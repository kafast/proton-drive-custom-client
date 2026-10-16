# Server-Image von protonsync: statisch gelinkte Binaries (musl), läuft nicht als root.
#
# Bauen (im Repository-Wurzelverzeichnis):
#   docker build -f packaging/container/Dockerfile -t protonsync .
# Betrieb: siehe README.md, Abschnitt „Erste Schritte (Server, Container)“

FROM rust:1-alpine AS build
# C-Werkzeuge für SQLite (gebündelt) und die TLS-Kryptografie (aws-lc).
# Python nur für die Sammlung der Lizenztexte.
RUN apk add --no-cache musl-dev gcc g++ make cmake perl linux-headers python3
WORKDIR /src
COPY . .
# Ohne Keyring: Auf dem Server liegen Geheimnisse verschlüsselt im Datenverzeichnis.
RUN cargo build --release --locked --no-default-features \
        -p protonsync-daemon -p protonsync-cli \
    && strip target/release/protonsyncd target/release/protonsync \
    && mkdir -p /out/data \
    && cargo metadata --format-version 1 --locked --no-default-features \
        --filter-platform "$(rustc -vV | sed -n 's/^host: //p')" \
       | python3 packaging/licenses/third_party.py protonsync-daemon protonsync-cli \
       > /out/THIRD-PARTY-LICENSES.txt

# Minimales Laufzeit-Image mit CA-Zertifikaten und Benutzer „nonroot“ (UID 65532).
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /src/target/release/protonsyncd /src/target/release/protonsync /usr/local/bin/
COPY --from=build /src/LICENSE.md /out/THIRD-PARTY-LICENSES.txt /usr/share/doc/protonsync/
# Leeres Datenverzeichnis, das dem Laufzeitbenutzer gehört; ein benanntes Volume übernimmt
# beim ersten Start diese Rechte (sonst gehörte es root und wäre nicht beschreibbar).
COPY --from=build --chown=65532:65532 /out/data /data
ENV PROTONSYNC_CONFIG=/config/config.toml \
    PROTONSYNC_DATA_DIR=/data \
    PROTONSYNC_SOCKET=/data/run/daemon.sock \
    PROTONSYNC_KEY_FILE=/run/secrets/protonsync-key
VOLUME ["/data"]
USER nonroot
ENTRYPOINT ["/usr/local/bin/protonsyncd"]
