# Build: docker build -t slm-router .
# Run:   docker run --rm -p 8080:8080 -e ROUTER_API_KEY=... -e OPENAI_API_KEY=... \
#          --add-host=host.docker.internal:host-gateway \
#          -v "$PWD/slm-router.toml:/etc/slm-router/slm-router.toml:ro" -v slm-router-log:/var/lib/slm-router \
#          slm-router
# Inside the container, point [local] base_url at http://host.docker.internal:11434/v1 and set
# [log] path = "/var/lib/slm-router/slm-router.jsonl" so the log survives restarts.
FROM rust:1-slim AS build
# reqwest's TLS (rustls with aws-lc-rs) compiles C code; cmake and clang cover its build script.
RUN apt-get update && apt-get install -y --no-install-recommends cmake clang \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo build --release --locked

FROM debian:stable-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --home /var/lib/slm-router --create-home slm-router
COPY --from=build /src/target/release/slm-router /usr/local/bin/slm-router
USER slm-router
WORKDIR /var/lib/slm-router
ENV ROUTER_CONFIG=/etc/slm-router/slm-router.toml
EXPOSE 8080
# A non-loopback bind is refused unless ROUTER_API_KEY is set, so pass it with -e.
ENTRYPOINT ["slm-router"]
CMD ["serve", "--bind", "0.0.0.0:8080"]
