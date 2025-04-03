FROM rust:1.86@sha256:563b33de55d0add224b2e301182660b59bf3cf7219e9dc0fda68f8500e5fe14a

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]