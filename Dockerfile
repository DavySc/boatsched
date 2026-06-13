FROM python:3.11-slim
WORKDIR /app

COPY pyproject.toml ./
COPY src/ ./src/
RUN pip install --no-cache-dir -e "."

# Non-root runtime user; owns the data directory.
RUN mkdir -p /data \
    && useradd --system --uid 10001 --home /app boatsched \
    && chown -R boatsched:boatsched /data /app
USER boatsched

EXPOSE 8000

ENV BOATSCHED_DB=/data/boatsched.db

# Binds 0.0.0.0 inside the container; compose maps it to 127.0.0.1 only.
# Auth hardening (require_auth, secure cookies) is enabled via env in compose.
CMD ["boatsched-server", "--db", "/data/boatsched.db", "--host", "0.0.0.0", "--port", "8000"]
