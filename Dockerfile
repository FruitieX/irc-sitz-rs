FROM rust:1.99@sha256:09027f873d45b8ecd1dc86f1fa13b7a62b875c961f017a0e331267b350430137

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]