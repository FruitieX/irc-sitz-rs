FROM rust:1.74@sha256:fd45a543ed41160eae2ce9e749e5b3c972625b0778104e8962e9bfb113535301

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]