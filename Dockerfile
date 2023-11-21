FROM rust:1.74@sha256:6de6071df133f8be44dd4538c74e93590941c6d2b9c98853e05011714fbcf57d

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]