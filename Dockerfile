FROM python:3.11-slim
WORKDIR /app

COPY pyproject.toml ./
COPY src/ ./src/
RUN pip install --no-cache-dir -e "."

# Create data directory
RUN mkdir -p /data

EXPOSE 8000

ENV BOATSCHED_DB=/data/boatsched.db

CMD ["boatsched-server", "--db", "/data/boatsched.db", "--host", "0.0.0.0", "--port", "8000"]
