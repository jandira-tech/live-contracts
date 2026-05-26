# SEC EX-10 backend as a Hugging Face Docker Space.
# Runs the unified worker (SEC listener + markdown backfill + internal FastAPI)
# and, when TUNNEL_TOKEN is set, a Cloudflare Tunnel exposing the API at a stable
# Cloudflare hostname. The Astro frontend on Cloudflare points SEC_API_URL there.
FROM python:3.13-slim

# cloudflared (Cloudflare Tunnel daemon) + curl/ca-certs
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
    && curl -fsSL https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64 \
         -o /usr/local/bin/cloudflared \
    && chmod +x /usr/local/bin/cloudflared \
    && apt-get purge -y curl && apt-get autoremove -y \
    && rm -rf /var/lib/apt/lists/*

# Non-root user (Hugging Face convention; uid 1000).
RUN useradd -m -u 1000 user
USER user
ENV HOME=/home/user PATH=/home/user/.local/bin:$PATH
WORKDIR /home/user/app

COPY --chown=user requirements.txt .
RUN pip install --no-cache-dir --upgrade pip && pip install --no-cache-dir -r requirements.txt

COPY --chown=user sec_listener ./sec_listener
COPY --chown=user seed.db ./seed.db
COPY --chown=user entrypoint.sh ./entrypoint.sh

ENV PYTHONPATH=/home/user/app \
    PYTHONUNBUFFERED=1 \
    SEC_SERVE_API=true \
    SEC_API_HOST=0.0.0.0 \
    SEC_API_PORT=7860 \
    SEC_RUN_HOURS=0 \
    SEC_CONVERT_MARKDOWN=true \
    SEC_DB_PATH=/home/user/app/data/ex10_listener.db

EXPOSE 7860
CMD ["bash", "entrypoint.sh"]
