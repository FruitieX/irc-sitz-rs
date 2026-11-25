FROM rust:1.100@sha256:e84068000361bf3d4a5ba2b7647f62893528b1eb9f847d7a23b493f5ce8ea254

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]