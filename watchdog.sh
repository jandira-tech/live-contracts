#!/bin/bash
# SEC Listener Watchdog - checks if running, restarts if not
# Runs hourly via cron

cd /home/arthrod/workspace/sec-listener

LOG_FILE="sec-listener.log"
PID_FILE="sec-listener.pid"
VENV_PYTHON=".venv/bin/python"
# The worker supervises BOTH the listener loop and the markdown backfill loop.
# Match the actual launch line (python -m ...), not the bare module string, so
# unrelated shell command lines that merely mention the module don't false-match.
PROC_PATTERN="python -m sec_listener.worker"

# Hardened worker runs as a module; config via env (defaults in config.py).
export SEC_RUN_HOURS="${SEC_RUN_HOURS:-0}"          # 0 = run until stopped; watchdog supervises
export SEC_CONVERT_MARKDOWN="${SEC_CONVERT_MARKDOWN:-true}"
# Serve the internal API in-process (bound to localhost — not public). One
# supervised process now covers listener + markdown backfill + API.
export SEC_SERVE_API="${SEC_SERVE_API:-true}"
export SEC_API_HOST="${SEC_API_HOST:-127.0.0.1}"
export SEC_API_PORT="${SEC_API_PORT:-8799}"

# Check if process is running via PID file
if [ -f "$PID_FILE" ]; then
    OLD_PID=$(cat "$PID_FILE")
    if kill -0 "$OLD_PID" 2>/dev/null; then
        echo "[$(date)] SEC listener running (PID $OLD_PID)" >> watchdog.log
        exit 0
    fi
fi

# Also check by process name as fallback
if pgrep -f "$PROC_PATTERN" > /dev/null 2>&1; then
    RUNNING_PID=$(pgrep -f "$PROC_PATTERN" | head -1)
    echo "$RUNNING_PID" > "$PID_FILE"
    echo "[$(date)] SEC listener running (PID $RUNNING_PID, recovered)" >> watchdog.log
    exit 0
fi

# Not running - restart it
echo "[$(date)] SEC listener NOT running. Restarting..." >> watchdog.log
# setsid fully detaches into its own session so it survives the launching shell.
setsid nohup "$VENV_PYTHON" -m sec_listener.worker >> "$LOG_FILE" 2>&1 < /dev/null &
NEW_PID=$!
echo "$NEW_PID" > "$PID_FILE"
sleep 3

# Verify it started
if kill -0 "$NEW_PID" 2>/dev/null; then
    echo "[$(date)] Restarted successfully (PID $NEW_PID)" >> watchdog.log
else
    echo "[$(date)] FAILED to restart! Check $LOG_FILE" >> watchdog.log
fi
