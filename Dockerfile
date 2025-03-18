FROM rust:1.85@sha256:4522a7efe3f50e61aa5a5c75546be42687819e488a1772ddc941136b7bc93848

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]