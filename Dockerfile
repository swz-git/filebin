FROM rust:1.78-slim-buster as builder
WORKDIR /usr/src/filebin
COPY . .
RUN cargo install --path .

FROM debian:buster-slim
RUN apt-get update && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/filebin /usr/local/bin/filebin
WORKDIR /usr/filebin
CMD ["filebin"]
