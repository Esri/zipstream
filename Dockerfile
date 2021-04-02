FROM rust:1.51.0-alpine@sha256:be03a93969087b1c610db2919b16a249fd38dc8942175ad811a221450ae356df as build

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
FROM alpine:3.13.4@sha256:ec14c7992a97fc11425907e908340c6c3d6ff602f5f13d899e6b7027c9b4133a
RUN apk add --no-cache libgcc
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
USER guest
CMD ["zipstream"]
EXPOSE 3000
