FROM rust:1.86@sha256:6a6dda669f020fa1fcb0903e37a049484fbf4b4699c8cb89db26ca030f475259

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]