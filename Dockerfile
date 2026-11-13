FROM rust:1.100@sha256:e19840879045c951c905d0eaa9887f3439c62000e429963371b376bac8d38c31

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]