FROM rust:1.43-alpine as build

# Build and cache dependencies
RUN apk add --no-cache musl-dev openssl-dev pkgconf make unzip python3
RUN mkdir -p /crate/src/ && echo 'fn main(){}' > /crate/src/main.rs
WORKDIR /crate
COPY Cargo.toml .
COPY Cargo.lock .

ENV RUSTFLAGS -C target-feature=-crt-static
RUN cargo build --release

# Build actual source
COPY src/* /crate/src/
RUN touch /crate/src/main.rs && cargo build --release

# Run tests
FROM build as test
RUN cargo test --release

# Deployment image
FROM alpine
RUN apk add --no-cache libgcc
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
CMD ["zipstream"]
EXPOSE 3000
