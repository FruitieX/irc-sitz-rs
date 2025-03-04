FROM rust:1.85@sha256:e15c642b487dd013b2e425d001d32927391aca787ac582b98cca72234d466b60

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]