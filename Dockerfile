FROM rust:1.84@sha256:ec7dae306d01d4c52d2b6cce4a62a8da2f2e54df543e527e1656ae7c4ef632b3

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]