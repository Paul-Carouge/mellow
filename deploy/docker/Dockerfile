# Mellow server image built from a signed release (ADR-0016), in the style of other self-hosted
# apps: the version is pinned in compose.yaml and `mellow-update` moves it forward.
FROM debian:trixie-slim AS download
ARG MELLOW_VERSION
ARG TARGETARCH
RUN apt-get update \
 && apt-get install --yes --no-install-recommends ca-certificates curl minisign \
 && rm -rf /var/lib/apt/lists/*
COPY minisign.pub /etc/mellow/minisign.pub
WORKDIR /download
RUN set -eu; \
    test -n "${MELLOW_VERSION}" || { echo "MELLOW_VERSION is required" >&2; exit 1; }; \
    case "${TARGETARCH:-amd64}" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu ;; \
      *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    asset="mellow-server-${MELLOW_VERSION}-${target}"; \
    base="https://github.com/Paul-Carouge/mellow/releases/download/v${MELLOW_VERSION}"; \
    curl --fail --silent --show-error --location --proto '=https' --remote-name "${base}/${asset}"; \
    curl --fail --silent --show-error --location --proto '=https' --remote-name "${base}/${asset}.minisig"; \
    # The trusted comment names the signed file, so a signature of another asset is refused.
    minisign -V -p /etc/mellow/minisign.pub -m "${asset}" | grep --quiet --line-regexp "Trusted comment: ${asset}"; \
    install -m 0755 "${asset}" /usr/local/bin/mellow-server

FROM debian:trixie-slim
# curl only serves the health check.
RUN apt-get update \
 && apt-get install --yes --no-install-recommends curl \
 && rm -rf /var/lib/apt/lists/*
COPY --from=download /usr/local/bin/mellow-server /usr/local/bin/mellow-server
USER 1000:1000
EXPOSE 8080
# Matches `listen_address` in mellow.toml; change it with the port.
ENV MELLOW_HEALTH_URL=http://127.0.0.1:8080/api/v1/health
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s \
  CMD curl --fail --silent --max-time 4 "${MELLOW_HEALTH_URL}" > /dev/null || exit 1
ENTRYPOINT ["mellow-server"]
CMD ["serve", "--config", "/config/mellow.toml"]
