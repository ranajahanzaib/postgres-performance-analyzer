FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src src
COPY demo/api/Cargo.toml demo/api/Cargo.toml
COPY demo/api/src demo/api/src
RUN cargo build --release -p pgpa-demo-api

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/pgpa-demo-api /usr/local/bin/pgpa-demo-api
USER 65532:65532
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/pgpa-demo-api"]
