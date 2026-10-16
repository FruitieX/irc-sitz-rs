FROM rust:1.99@sha256:e259494b14672950a909b4c54527e19d29cae6ebf980672114c8bb85e5594e4a

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]