FROM rust:1.88@sha256:1928f85f204effc91fddc53875afd042b651552fde6ee11acaafde641942dd70

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]