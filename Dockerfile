FROM rust:1.98@sha256:c92b3414e418f5b250f13c5bd393f90192f0b4bb4767e64f29c9e583197b27cb

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]