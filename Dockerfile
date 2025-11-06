FROM golang:latest as builder
ARG CGO_ENABLED=0

COPY ./ /root/src/
WORKDIR /root/src/
ARG VERSION=""
ARG BUILD_DATE=""
RUN set -eux; \
    base=${VERSION:-$(git describe --tags --abbrev=0 || echo dev)}; \
    date=${BUILD_DATE:-$(date +%Y%m%d)}; \
    sha=$(git rev-parse --short=7 HEAD || echo nogithash); \
    v="$base-$date-$sha"; \
    go build -ldflags "-s -w -X main.version=$v" -trimpath -o mosdns

FROM alpine:latest

COPY --from=builder /root/src/mosdns /usr/bin/

RUN apk add --no-cache ca-certificates
