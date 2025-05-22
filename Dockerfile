FROM rust:1.87@sha256:893a03d96de2c0e6eb8b73e24b2266264b6dee244ce997d1bae246901093ba23

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]