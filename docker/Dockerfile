# syntax=docker/dockerfile:1

FROM rust:1.99.1-slim-trixie AS build
ARG WASM_BINDGEN=0.2.129
RUN apt-get update \
 && apt-get install -y --no-install-recommends binaryen brotli \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown \
 && cargo install --locked wasm-bindgen-cli --version "$WASM_BINDGEN" --root /usr/local/wasm-bindgen
WORKDIR /src
COPY Cargo.toml Cargo.lock LICENSE-MIT LICENSE-APACHE ./
COPY crates crates
COPY tools tools
COPY scripts/web/client.html scripts/web/client.js scripts/web/stamp.sh scripts/web/
COPY docker/third-party-licenses.sh docker/
RUN cargo build --release --locked -p mandrake-server --bin mandrake-server --bin mandrake-ws-proxy \
 && mkdir -p /out \
 && cp target/release/mandrake-server target/release/mandrake-ws-proxy /out/
ARG REVISION=unknown
ARG REVISION_DATE=
RUN M2_REVISION="$REVISION" M2_REVISION_DATE="$REVISION_DATE" \
    cargo build --release --locked --target wasm32-unknown-unknown -p mandrake-web --lib \
 && mkdir -p /out/web/js \
 && /usr/local/wasm-bindgen/bin/wasm-bindgen --target web --out-dir /out/web/pkg \
      target/wasm32-unknown-unknown/release/mandrake_web.wasm \
 && wasm-opt -O2 --mvp-features --enable-bulk-memory --enable-nontrapping-float-to-int \
      --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
      /out/web/pkg/mandrake_web_bg.wasm -o /out/web/pkg/mandrake_web_bg.opt.wasm \
 && mv /out/web/pkg/mandrake_web_bg.opt.wasm /out/web/pkg/mandrake_web_bg.wasm \
 && for f in /out/web/pkg/*.wasm /out/web/pkg/*.js; do \
      gzip -9 -n -k "$f" && brotli -q 11 -k "$f"; \
    done \
 && cp scripts/web/client.html /out/web/index.html \
 && cp scripts/web/client.js /out/web/ \
 && cp crates/mandrake-client/mandrake.original.toml crates/mandrake-client/mandrake.modernretro.toml /out/web/ \
 && cp crates/mandrake-web/js/*.js /out/web/js/ \
 && sh scripts/web/stamp.sh /out/web \
 && sh docker/third-party-licenses.sh >/out/THIRD-PARTY-LICENSES

FROM debian:trixie-slim
ARG REVISION=unknown
LABEL org.opencontainers.image.title="myth2-server" \
      org.opencontainers.image.licenses="MIT OR Apache-2.0" \
      org.opencontainers.image.revision="$REVISION"
RUN groupadd -g 1000 myth2 && useradd -u 1000 -g 1000 -M -d /nonexistent -s /usr/sbin/nologin myth2 \
 && mkdir -p /data/myth2 /data/plugins /data/tfl /config /films /logs /etc/myth2 /usr/share/doc/myth2-server \
 && chown 1000:1000 /films /logs
COPY --from=build /out/mandrake-server /out/mandrake-ws-proxy /usr/local/bin/
COPY --from=build /out/THIRD-PARTY-LICENSES /usr/share/doc/myth2-server/
COPY --from=build /out/web /usr/share/myth2/web
COPY LICENSE-MIT LICENSE-APACHE /usr/share/doc/myth2-server/
COPY docker/server.toml docker/tfl-server.toml docker/proxy.toml docker/proxy-site.toml /etc/myth2/
COPY docker/entrypoint.sh /usr/local/bin/myth2-entrypoint
ENV NO_COLOR=1
USER 1000:1000
EXPOSE 3453/tcp 3454/tcp 8080/tcp
ENTRYPOINT ["/usr/local/bin/myth2-entrypoint"]
