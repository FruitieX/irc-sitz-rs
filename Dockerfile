FROM rust:1.84@sha256:f7cbb35003d4ffb5543f8ad6480c1e36bbae5c3609523c9f0c2e223668ee9c1a

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]