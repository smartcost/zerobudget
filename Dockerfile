FROM rust:1.98.1-bookworm AS build
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --locked --release -p obz-api -p obz-infrastructure --bin obz-api --example migrate --example auth_admin
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home zerobudget
COPY --from=build /app/target/release/obz-api /usr/local/bin/obz-api
COPY --from=build /app/target/release/examples/migrate /usr/local/bin/zb-migrate
COPY --from=build /app/target/release/examples/auth_admin /usr/local/bin/zb-auth-admin
USER 10001:10001
ENV API_BIND=0.0.0.0:3000
EXPOSE 3000
ENTRYPOINT ["obz-api"]
