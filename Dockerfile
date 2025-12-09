# Build stage
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev protobuf-dev protoc

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY proto/ proto/
COPY core/ core/
COPY cli/ cli/
COPY daemon/ daemon/

RUN cargo build --release --package ross-daemon

# Runtime stage
FROM alpine:latest

RUN apk add --no-cache ca-certificates

RUN addgroup -S ross && adduser -S ross -G ross

COPY --from=builder /app/target/release/ross-daemon /usr/local/bin/ross-daemon

RUN chown ross:ross /usr/local/bin/ross-daemon

USER ross

EXPOSE 50051

ENTRYPOINT ["ross-daemon", "start"]
