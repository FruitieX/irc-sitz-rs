FROM rust:1.87@sha256:251cec8da4689d180f124ef00024c2f83f79d9bf984e43c180a598119e326b84

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]