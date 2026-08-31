# Build Stage
FROM rust:1.98-alpine AS builder
WORKDIR /usr/src/
# Install required build dependencies
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static gcc g++ make

# - Install dependencies
WORKDIR /usr/src
RUN USER=root cargo new watchtower
WORKDIR /usr/src/watchtower
COPY Cargo.toml Cargo.lock ./
RUN cargo build --release

# - Copy source
COPY src ./src
RUN touch src/main.rs src/lib.rs && cargo build --release

# ---- Runtime Stage ----
FROM alpine:latest AS runtime
COPY --from=builder /usr/src/watchtower/target/release/watchtower /usr/local/bin/watchtower
USER 1000
CMD ["watchtower"]
