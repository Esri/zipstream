FROM rust:1.59.0-alpine@sha256:a8168e9102a0b338fa8b683733cc58bcc9fb2de2d4ed1198f9bff22a9ae88478 as base

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
