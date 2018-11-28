FROM rust:1.30.1 as build

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

# Deployment image
FROM buildpack-deps:curl
COPY --from=build /crate/target/release/zipstream /usr/local/bin/
CMD ["zipstream"]
EXPOSE 3000
