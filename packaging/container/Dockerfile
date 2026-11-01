# Server image of protonsync: statically linked binaries (musl), does not run as root.
#
# Build (in the repository root):
#   docker build -f packaging/container/Dockerfile -t protonsync .
# Operation: see README.md, section „Erste Schritte (Server, Container)“

FROM rust:1-alpine AS build
# C tools for SQLite (bundled) and the TLS cryptography (aws-lc).
# Python only for collecting the license texts.
RUN apk add --no-cache musl-dev gcc g++ make cmake perl linux-headers python3
WORKDIR /src
COPY . .
# Without keyring: on the server, secrets are stored encrypted in the data directory.
RUN cargo build --release --locked --no-default-features \
        -p protonsync-daemon -p protonsync-cli \
    && strip target/release/protonsyncd target/release/protonsync \
    && mkdir -p /out/data \
    && cargo metadata --format-version 1 --locked --no-default-features \
        --filter-platform "$(rustc -vV | sed -n 's/^host: //p')" \
       | python3 packaging/licenses/third_party.py protonsync-daemon protonsync-cli \
       > /out/THIRD-PARTY-LICENSES.txt

# SDK helper (ADR 0025): Proton's official SDK compiled with Bun into one file.
# Bun needs glibc and libstdc++, hence the "cc" runtime image below.
FROM oven/bun:1.4.2-debian AS helper
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml ./
COPY helper ./helper
RUN helper/build.sh bun-linux-x64-baseline

# Minimal runtime image with glibc/libstdc++, CA certificates and the user
# "nonroot" (UID 65532).
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/protonsyncd /src/target/release/protonsync /usr/local/bin/
COPY --from=helper /src/helper/dist/protonsync-sdk-helper /usr/local/bin/
COPY --from=build /src/LICENSE.md /out/THIRD-PARTY-LICENSES.txt /usr/share/doc/protonsync/
COPY --from=helper /src/helper/vendor/sdk/LICENSE.md /usr/share/doc/protonsync/proton-drive-sdk-LICENSE.md
# Empty data directory owned by the runtime user; a named volume takes over these
# permissions on first start (otherwise it would belong to root and not be writable).
COPY --from=build --chown=65532:65532 /out/data /data
ENV PROTONSYNC_CONFIG=/config/config.toml \
    PROTONSYNC_DATA_DIR=/data \
    PROTONSYNC_SOCKET=/data/run/daemon.sock \
    PROTONSYNC_KEY_FILE=/run/secrets/protonsync-key
VOLUME ["/data"]
USER nonroot
ENTRYPOINT ["/usr/local/bin/protonsyncd"]
