FROM rust:bookworm AS build
COPY ./src ./src
COPY ./Cargo.lock .
COPY ./Cargo.toml .

#ENV PGOONLY=y
#RUN rustup component add llvm-tools-preview && \
#  cargo install cargo-pgo && \
#  cargo pgo build && \
#  cargo pgo run && \
#  cargo pgo optimize
RUN cargo b -r

FROM debian:bookworm-slim AS mensa-api
RUN apt-get update && \
  apt-get install -y --no-install-recommends \
  libsqlite3-0 ca-certificates \
  && \
  apt-get autoremove -y && \
  apt-get clean -y && \
  rm -rf /var/lib/apt/lists/*

# COPY --from=build ./target/*/release/mensa-api /app/mensa-api
COPY --from=build ./target/release/mensa-api /app/mensa-api
WORKDIR /app/data
EXPOSE 9090
ENTRYPOINT ["/app/mensa-api"]
