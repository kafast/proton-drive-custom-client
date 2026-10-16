# Server-Image von protonsync: statisch gelinkte Binaries (musl), läuft nicht als root.
#
# Bauen (im Repository-Wurzelverzeichnis):
#   docker build -f packaging/container/Dockerfile -t protonsync .
# Betrieb: siehe packaging/container/README.md

FROM rust:1-alpine AS build
# C-Werkzeuge für SQLite (gebündelt) und die TLS-Kryptografie (aws-lc).
RUN apk add --no-cache musl-dev gcc g++ make cmake perl linux-headers
WORKDIR /src
COPY . .
# Ohne Keyring: Auf dem Server liegen Geheimnisse verschlüsselt im Datenverzeichnis.
RUN cargo build --release --locked --no-default-features \
        -p protonsync-daemon -p protonsync-cli \
    && strip target/release/protonsyncd target/release/protonsync

# Minimales Laufzeit-Image mit CA-Zertifikaten und Benutzer „nonroot“ (UID 65532).
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /src/target/release/protonsyncd /src/target/release/protonsync /usr/local/bin/
COPY --from=build /src/LICENSE.md /usr/share/doc/protonsync/LICENSE.md
ENV PROTONSYNC_CONFIG=/config/config.toml \
    PROTONSYNC_DATA_DIR=/data \
    PROTONSYNC_SOCKET=/data/run/daemon.sock \
    PROTONSYNC_KEY_FILE=/run/secrets/protonsync-key
VOLUME ["/data"]
USER nonroot
ENTRYPOINT ["/usr/local/bin/protonsyncd"]
