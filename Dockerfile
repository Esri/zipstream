FROM rust:1.50.0-alpine@sha256:5280ec3be86b0b81da654e7c019ab3ec10a7e7e8556dfe5e02bdf2276d11be04 as build

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
FROM alpine:3.13.1@sha256:08d6ca16c60fe7490c03d10dc339d9fd8ea67c6466dea8d558526b1330a85930
RUN apk add --no-cache libgcc
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
USER guest
CMD ["zipstream"]
EXPOSE 3000
