FROM rust:1.96 AS rust-builder
WORKDIR /workspace
COPY rust/ ./rust/
RUN cargo build --manifest-path rust/Cargo.toml --release

FROM python:3.13-slim AS runtime
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY src/python_app ./python_app
COPY --from=rust-builder /workspace/rust/target/release/worker /usr/local/bin/worker
ENV RIDGELINE_WORKER=/usr/local/bin/worker
EXPOSE 8080
CMD ["python", "-m", "python_app.service"]

