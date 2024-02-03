FROM rust:1.75@sha256:87f3b2f93b82995443a1a558c234212dafe79cfdc3af956539610560369ddcd0

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]