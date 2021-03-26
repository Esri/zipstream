FROM rust:1.51.0-alpine@sha256:abdc45fab929afffd86fb418c0da0591324bfe8bd8263a73e341fdf5d65a250f as build

# Build and cache dependencies
RUN apk add --no-cache musl-dev openssl-dev pkgconf make unzip python3
RUN mkdir -p /crate/src/ && echo 'fn main(){}' > /crate/src/main.rs
WORKDIR /crate
COPY Cargo.toml .
COPY Cargo.lock .

ENV RUSTFLAGS -C target-feature=-crt-static
RUN cargo build --locked --release

# Build actual source
COPY src/* /crate/src/
RUN touch /crate/src/main.rs && cargo build --locked --release

# Run tests
FROM build as test
RUN cargo test --release

# Deployment image
FROM alpine:3.13.3@sha256:826f70e0ac33e99a72cf20fb0571245a8fee52d68cb26d8bc58e53bfa65dcdfa
RUN apk add --no-cache libgcc
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
USER guest
CMD ["zipstream"]
EXPOSE 3000
