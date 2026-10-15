# syntax=docker/dockerfile:1.7
# @impl SPEC_OPS_IMAGE

FROM rust:1.98-trixie AS build
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p zaehler-server \
 && cp target/release/zaehler /zaehler \
 && mkdir -p /out/var/lib/zaehler/data/files

FROM gcr.io/distroless/cc-debian13:nonroot
COPY --from=build /zaehler /usr/local/bin/zaehler
COPY --from=build --chown=65532:65532 /out/var/lib/zaehler /var/lib/zaehler
WORKDIR /var/lib/zaehler
ENV ZAEHLER_LOG__FORMAT=json
EXPOSE 8080 9090
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/zaehler"]
CMD ["serve"]
