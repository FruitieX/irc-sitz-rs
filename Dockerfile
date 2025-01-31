FROM rust:1.84@sha256:efe14eee1be3fd2462fe349b5948b0d1b179b421c9fb23acb20b579f59299daf

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]