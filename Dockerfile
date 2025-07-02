FROM rust:1.88@sha256:6b964127aa8552b6aca53db0c12ba7c51a1f59d9e9990c74e77c11e83ff6a08e

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]