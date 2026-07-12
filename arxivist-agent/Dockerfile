FROM ghcr.io/astral-sh/uv:python3.14-bookworm-slim
WORKDIR /app
COPY arxivist-agent/pyproject.toml arxivist-agent/uv.lock ./
RUN uv sync --frozen --no-dev
COPY arxivist-agent/ ./
ENV PATH="/app/.venv/bin:${PATH}"
EXPOSE 8000
CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8000"]
