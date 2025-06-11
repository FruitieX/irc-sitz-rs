FROM rust:1.87@sha256:b571d7b2dc7b9154517eeda87c0c3c97865d432e95ec205e34a194fd2baaff1d

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]