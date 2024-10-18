FROM rust:1.82.0-alpine@sha256:466dc9924d265455aa73e72fd9cdac9db69ce6a988e6f0e6baf852db3485d97d as base

FROM base as build

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

# Use APK to assemble a root filesystem with very select dependencies,
# not even busybox or apk (basically a custom "distroless")
FROM base as chroot
RUN apk add --no-cache --root /chroot --initdb \
            --keys-dir /etc/apk/keys --repositories-file /etc/apk/repositories \
            libgcc openssl
# ca-certificates package pulls in busybox, so just copy the output
RUN mkdir -p /chroot/etc/ssl/certs/ && cp /etc/ssl/certs/ca-certificates.crt /chroot/etc/ssl/certs/ca-certificates.crt

# Deployment image
FROM scratch
COPY --from=chroot /chroot /
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
USER 2000
CMD ["zipstream"]
EXPOSE 3000
LABEL log-format="tracing-subscriber-json"
