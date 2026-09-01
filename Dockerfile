FROM rust:1.98@sha256:620dbcd124499c59e2406d3741574b5c5838cf9eb9656f0c3a03948f79b02959

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]