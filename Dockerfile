FROM rust:1.93@sha256:e35d0f677e0e0be6f4b4f93bf16e6f93ab4f427dc440a0ef12511026f8b7d7e3

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]