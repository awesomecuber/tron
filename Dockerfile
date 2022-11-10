FROM lukemathwalker/cargo-chef:latest-rust-1.65.0 as chef
WORKDIR /app
RUN apt update && apt install lld clang -y

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN rustup target add wasm32-unknown-unknown
RUN cargo install -f wasm-bindgen-cli
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release
RUN wasm-bindgen --out-dir tron --target web ./target/wasm32-unknown-unknown/release/tron.wasm

FROM nginx
COPY --from=builder /app/tron /usr/share/nginx/html/tron
COPY index.html /usr/share/nginx/html