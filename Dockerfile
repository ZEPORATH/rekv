FROM rustlang/rust:nightly-bullseye-slim AS build

WORKDIR /app

COPY . .

RUN cargo build --release

FROM debian:bullseye-slim

WORKDIR /app

COPY --from=build /app/target/release/rekv /usr/local/bin/rekv

CMD ["rekv"]
