FROM rust:1.86@sha256:ff735b1f09be7bb43d0ceece3d6f03b877292ae0307e35b32f75165a05d574c5

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]