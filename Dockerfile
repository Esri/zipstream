FROM rust:1.51.0-alpine@sha256:a84599229afe057f68257e357bcd22d3c994b12b537b6f8e8e09852a29e6f785 as base

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
RUN cp /etc/ssl/certs/ca-certificates.crt /chroot/etc/ssl/certs/ca-certificates.crt

# Deployment image
<<<<<<< HEAD
FROM alpine:3.13.5@sha256:69e70a79f2d41ab5d637de98c1e0b055206ba40a8145e7bddb55ccc04e13cf8f
RUN apk add --no-cache libgcc
||||||| parent of decace6 (docker: use apk to build a custom "distroless" root fs for the image)
FROM alpine:3.13.3@sha256:826f70e0ac33e99a72cf20fb0571245a8fee52d68cb26d8bc58e53bfa65dcdfa
RUN apk add --no-cache libgcc
=======
FROM scratch
COPY --from=chroot /chroot /
>>>>>>> decace6 (docker: use apk to build a custom "distroless" root fs for the image)
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
USER 2000
CMD ["zipstream"]
EXPOSE 3000
