FROM rust:1.84@sha256:4ac764e7954b5a31cb4ca1df31885497cf86ed532ea1c4456da1b9a960964eef

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]