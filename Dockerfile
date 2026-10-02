FROM rust:1.99@sha256:cb1b90b0ce00f9eb950c4de62cc1bb89c7bf8a3577de6600d10e31fb15f876de

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]