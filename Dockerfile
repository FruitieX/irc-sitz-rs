FROM rust:1.99@sha256:6ff07edce8775d0f64be7aba9197229407301bddf2054d62c27b541a6238a181

RUN apt-get update
RUN apt-get install -y libclang-dev libespeak-ng-libespeak-dev python3

WORKDIR /app
COPY . .

RUN cargo install --path .

EXPOSE 7878
CMD ["irc-sitz-rs"]