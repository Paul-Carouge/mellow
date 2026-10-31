#!/bin/sh
# Runs the load generator against a fresh local SFU for each room, as "participants:speakers".
# Usage: ./run-series.sh [room ...]   (default: 2:2 5:5 10:10 25:25 50:50 50:5 100:5)
set -eu
rooms=${*:-"2:2 5:5 10:10 25:25 50:50 50:5 100:5"}
for room in $rooms; do
  bots=${room%:*}
  speakers=${room#*:}
  ./target/release/sfu --udp-port 50000 --http-port 3000 >sfu.log 2>&1 &
  sfu_pid=$!
  sleep 1
  ip=$(sed -nE 's#.*http://([0-9.]+):.*#\1#p' sfu.log | head -1)
  echo "== $bots participants, $speakers speaking"
  ./target/release/load --server "http://$ip:3000" --bots "$bots" --speakers "$speakers" ${SLOTS:+--slots $SLOTS} --seconds 20 | tail -1
  grep "in room" sfu.log | tail -2
  kill "$sfu_pid"
  wait "$sfu_pid" 2>/dev/null || true
  sleep 1
done
rm -f sfu.log
