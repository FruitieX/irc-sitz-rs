FROM rust:1.87@sha256:25038aa450210c53cf05dbf7b256e1df1ee650a58bb46cbc7d6fa79c1d98d083

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]