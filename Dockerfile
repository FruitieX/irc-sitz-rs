FROM rust:1.74@sha256:44dd40cdaf3654dc1304163dc66c99200ada94d03ccc18182ef119fbcca2c761

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]