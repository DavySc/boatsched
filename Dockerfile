# Stage 1: Build React frontend
FROM node:20-alpine AS frontend-build
WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json* ./
RUN npm ci --legacy-peer-deps
COPY frontend/ ./
RUN npm run build

# Stage 2: Python backend
FROM python:3.11-slim
WORKDIR /app

# Install Python dependencies
COPY pyproject.toml ./
COPY src/ ./src/
RUN pip install --no-cache-dir -e "."

# Copy built frontend
COPY --from=frontend-build /app/frontend/dist ./frontend/dist

# Create data directory
RUN mkdir -p /data

EXPOSE 8000

ENV BOATSCHED_DB=/data/boatsched.db

CMD ["boatsched-server", "--db", "/data/boatsched.db", "--host", "0.0.0.0", "--port", "8000"]
