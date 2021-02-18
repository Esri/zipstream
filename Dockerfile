FROM rust:1.50.0-alpine@sha256:992727fec8dd4be79f3d96958a6315ff49a023812a127271d04f97d896ec4a90 as build

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
FROM alpine:3.13.2@sha256:a75afd8b57e7f34e4dad8d65e2c7ba2e1975c795ce1ee22fa34f8cf46f96a3be
RUN apk add --no-cache libgcc
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
USER guest
CMD ["zipstream"]
EXPOSE 3000
