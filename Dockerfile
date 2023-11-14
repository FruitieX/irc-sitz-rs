FROM rust:1.73@sha256:25fa7a9aa4dadf6a466373822009b5361685604dbe151b030182301f1a3c2f58

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]