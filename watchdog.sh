#!/bin/bash
# SEC Listener Watchdog - checks if running, restarts if not
# Runs hourly via cron

cd /home/arthrod/workspace/sec-listener

LOG_FILE="sec-listener.log"
PID_FILE="sec-listener.pid"
VENV_PYTHON=".venv/bin/python"
MAIN_SCRIPT="sec-listener.py"

# Check if process is running via PID file
if [ -f "$PID_FILE" ]; then
    OLD_PID=$(cat "$PID_FILE")
    if kill -0 "$OLD_PID" 2>/dev/null; then
        echo "[$(date)] SEC listener running (PID $OLD_PID)" >> watchdog.log
        exit 0
    fi
fi

# Also check by process name as fallback
if pgrep -f "sec-listener.py" > /dev/null 2>&1; then
    RUNNING_PID=$(pgrep -f "sec-listener.py")
    echo "$RUNNING_PID" > "$PID_FILE"
    echo "[$(date)] SEC listener running (PID $RUNNING_PID, recovered)" >> watchdog.log
    exit 0
fi

# Not running - restart it
echo "[$(date)] SEC listener NOT running. Restarting..." >> watchdog.log
nohup "$VENV_PYTHON" "$MAIN_SCRIPT" >> "$LOG_FILE" 2>&1 &
NEW_PID=$!
echo "$NEW_PID" > "$PID_FILE"
sleep 3

# Verify it started
if kill -0 "$NEW_PID" 2>/dev/null; then
    echo "[$(date)] Restarted successfully (PID $NEW_PID)" >> watchdog.log
else
    echo "[$(date)] FAILED to restart! Check $LOG_FILE" >> watchdog.log
fi
