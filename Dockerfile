FROM rust:1.86@sha256:7b65306dd21304f48c22be08d6a3e41001eef738b3bd3a5da51119c802321883

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]