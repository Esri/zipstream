FROM rust:1.42.0 as build

# Build and cache dependencies
RUN apt-get update && apt-get install unzip
RUN mkdir -p /crate/src/ && echo 'fn main(){}' > /crate/src/main.rs
WORKDIR /crate
COPY Cargo.toml .
COPY Cargo.lock .
RUN cargo build --release

# Build actual source
COPY src/* /crate/src/
RUN touch /crate/src/main.rs && cargo build --release

# Run tests
FROM build as test
RUN cargo test --release

# Deployment image
FROM ubuntu:bionic
RUN apt-get update && apt-get install -y libssl1.1 ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=build /crate/target/release/zipstream /usr/local/bin/
CMD ["zipstream"]
EXPOSE 3000
