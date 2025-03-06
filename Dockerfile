FROM rust:1.85@sha256:80ccfb51023dbb8bfa7dc469c514a5a66343252d5e7c5aa0fab1e7d82f4ebbdc

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]