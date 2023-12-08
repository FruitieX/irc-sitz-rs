FROM rust:1.74@sha256:32d220ca8c77fe56afd6d057c382ea39aced503278526a34fc62b90946f92e02

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]