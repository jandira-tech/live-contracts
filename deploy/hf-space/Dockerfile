# SEC EX-10 backend as a Hugging Face Docker Space.
# Runs the unified worker (SEC listener + markdown backfill + internal FastAPI on
# :7860). Hugging Face exposes it publicly at the Space URL; the Astro frontend on
# Cloudflare points SEC_API_URL there and caches it at the edge (CDN + SWR).
# NOTE: cloudflared/Cloudflare Tunnel is intentionally NOT used — Hugging Face's
# abuse policy bans it on Spaces. HF already provides the public HTTPS endpoint.
FROM python:3.13-slim

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
