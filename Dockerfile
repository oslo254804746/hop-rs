FROM rust:1.90-bookworm AS build

WORKDIR /src
COPY . .
ARG SERVER_BIN=hop-server
RUN cargo build --release -p hop-server \
    && install -m 0755 "target/release/${SERVER_BIN}" /usr/local/bin/hop-server

FROM debian:bookworm-slim

RUN useradd --system --create-home --home-dir /var/lib/hop hop
WORKDIR /var/lib/hop

COPY --from=build /usr/local/bin/hop-server /usr/local/bin/hop-server
COPY config.example.toml /etc/hop/config.toml

RUN mkdir -p /etc/hop /var/lib/hop && chown -R hop:hop /var/lib/hop

USER hop
EXPOSE 2222

CMD ["hop-server", "serve", "--config", "/etc/hop/config.toml"]
