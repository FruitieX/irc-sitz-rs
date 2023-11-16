FROM rust:1.74@sha256:47045897a478b674df2213f0bf27b059dbfc4129328240e22529658d7dd7bd28

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]