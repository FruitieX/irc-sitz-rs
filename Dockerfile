FROM rust:1.75@sha256:e17a45360b8569720da89dd7bf3c8628ba801c0758c8b4f12b1b32a9327a43a7

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]