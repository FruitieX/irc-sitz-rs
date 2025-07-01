FROM rust:1.88@sha256:749d5f12aa5f38ebf81012a0385b8e6adcb7b6e8f494961d559e8a7264803d4f

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]