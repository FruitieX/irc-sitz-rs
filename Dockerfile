FROM rust:1.86@sha256:300ec56abce8cc9448ddea2172747d048ed902a3090e6b57babb2bf19f754081

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]