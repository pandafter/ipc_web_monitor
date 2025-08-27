FROM rustlang/rust:nightly as builder
WORKDIR /app
COPY . .
RUN cargo build --release --bin data_hub

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/data_hub /usr/local/bin/data_hub
COPY web ./web
CMD ["data_hub"]
