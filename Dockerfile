FROM rust:1.90-bookworm AS build

WORKDIR /src
COPY . .
ARG SERVER_BIN=hop-server
RUN cargo build --release -p hop-server \
    && install -m 0755 "target/release/${SERVER_BIN}" /usr/local/bin/hop-server

FROM debian:bookworm-slim

RUN useradd --system --create-home --home-dir /data hop \
    && mkdir -p /usr/share/hop \
    && chown -R hop:hop /data
WORKDIR /data

COPY --from=build /usr/local/bin/hop-server /usr/local/bin/hop-server
COPY config.docker.toml /usr/share/hop/config.docker.toml
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

RUN chmod 0755 /usr/local/bin/docker-entrypoint.sh

VOLUME ["/data"]
EXPOSE 2222 8080

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["hop-server", "serve", "--config", "/data/config.toml"]
