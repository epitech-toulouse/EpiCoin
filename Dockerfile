# syntax=docker/dockerfile:1.7
# The EpiCoin binaries (node, API, CLI) in one small runtime image.

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked --bins \
 && mkdir /out \
 && cp target/release/epicoin-node target/release/epicoin-api target/release/epicoin-cli /out/

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/*
# Every directory a volume is mounted on exists, owned by the service user, so that new
# volumes inherit that ownership.
RUN useradd --system --uid 10001 --user-group --home-dir /nonexistent --shell /usr/sbin/nologin epicoin \
 && mkdir -p /data /chain /uploads /secrets/node /secrets/api \
 && chown -R epicoin:epicoin /data /chain /uploads /secrets
COPY --from=build /out/ /usr/local/bin/
COPY docker/epicoin-env docker/epicoin-init /usr/local/bin/
USER epicoin
ENTRYPOINT ["epicoin-env"]
CMD ["epicoin-cli", "--help"]
