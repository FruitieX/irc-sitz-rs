FROM rust:1.86@sha256:640960fe15de2f67cc88db7f0f547977cb759cba9eab246df29d98d02aaf24b8

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]