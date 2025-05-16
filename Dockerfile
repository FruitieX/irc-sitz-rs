FROM rust:1.87@sha256:5e33ae75f40bf25854fa86e33487f47075016d16726355a72171f67362ad6bf7

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]