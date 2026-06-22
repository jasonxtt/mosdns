FROM node:22-alpine AS webui-builder

WORKDIR /src/webui-log
COPY webui-log/package*.json ./
RUN npm ci

WORKDIR /src
COPY . /src

WORKDIR /src/webui-log
ARG MOSDNS_ASSET_VERSION=""
RUN set -eux; \
    export MOSDNS_ASSET_VERSION; \
    npm run build; \
    npm run build:log1

FROM golang:1.26 AS builder
ARG CGO_ENABLED=0

WORKDIR /src
COPY . /src
COPY --from=webui-builder /src/coremain/www/assets/vue-log /src/coremain/www/assets/vue-log
COPY --from=webui-builder /src/coremain/www/assets/vue-log1 /src/coremain/www/assets/vue-log1
COPY --from=webui-builder /src/coremain/www/log.html /src/coremain/www/log.html
COPY --from=webui-builder /src/coremain/www/log1.html /src/coremain/www/log1.html

ARG VERSION=""
ARG BUILD_DATE=""
ARG VCS_REF=""
RUN set -eux; \
    base=${VERSION:-dev}; \
    date=${BUILD_DATE:-$(date +%Y%m%d)}; \
    sha=${VCS_REF:-nogithash}; \
    v="$base-$date-$sha"; \
    go build -ldflags "-s -w -X main.version=$v" -trimpath -o /out/mosdns

FROM alpine:3.22

ARG VERSION=""
ARG BUILD_DATE=""
ARG VCS_REF=""

RUN apk add --no-cache ca-certificates tzdata

LABEL org.opencontainers.image.title="mosdns" \
      org.opencontainers.image.description="mosdns container image for the maintained WebUI fork" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.created="${BUILD_DATE}" \
      org.opencontainers.image.revision="${VCS_REF}"

ENV MOSDNS_CONTAINER_MODE=1 \
    MOSDNS_CONTAINER_NETWORK_MODE=bridge
WORKDIR /cus/mosdns
VOLUME ["/cus/mosdns"]

COPY --from=builder /out/mosdns /usr/bin/mosdns

EXPOSE 53/tcp 53/udp 9099/tcp

ENTRYPOINT ["/usr/bin/mosdns", "start", "-d", "/cus/mosdns", "-c", "/cus/mosdns/config_custom.yaml"]
