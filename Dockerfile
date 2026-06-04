FROM rust:1.90-bookworm AS build

WORKDIR /src
COPY . .
RUN cargo build --release --bin hop-server --bin hop

FROM debian:bookworm-slim

RUN useradd --system --create-home --home-dir /var/lib/hop hop
WORKDIR /var/lib/hop

COPY --from=build /src/target/release/hop-server /usr/local/bin/hop-server
COPY --from=build /src/target/release/hop /usr/local/bin/hop
COPY config.example.toml /etc/hop/config.toml

RUN mkdir -p /etc/hop /var/lib/hop && chown -R hop:hop /var/lib/hop

USER hop
EXPOSE 2222

CMD ["hop-server", "serve", "--config", "/etc/hop/config.toml"]
