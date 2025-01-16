FROM rust:1.84@sha256:e6e40c05cfe7dd55ad13794333d31b6d0818f0c6086876e7dc65871e6c8c0b21

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]