FROM rust:1.85@sha256:9285bed250441d504928a5bc1ad4694a9acc5d1873e72cb9d12b8e1dc2de6ee5

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]