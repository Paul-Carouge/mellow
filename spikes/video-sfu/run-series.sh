#!/bin/sh
# Runs the load generator against a fresh local SFU for each room, as "participants:cameras".
# Usage: [LAYERS=h:1500,l:500,q:150] [FOCUS=h] [OTHER=q] [SLOTS=9] [BWE=4000] [DURATION=20] \
#        ./run-series.sh [room ...]   (default: 2:2 5:5 10:5 25:10)
# LAYERS with one entry (the default, h:1500) means no simulcast; FOCUS and OTHER then stay unset.
set -eu
rooms=${*:-"2:2 5:5 10:5 25:10"}
for room in $rooms; do
  bots=${room%:*}
  cameras=${room#*:}
  ./target/release/sfu --udp-port 50000 --http-port 3000 \
    ${FOCUS:+--focus-layer $FOCUS} ${OTHER:+--other-layer $OTHER} \
    ${BWE:+--bwe-desired-kbps $BWE} >sfu.log 2>&1 &
  sfu_pid=$!
  sleep 1
  ip=$(sed -nE 's#.*http://([0-9.]+):.*#\1#p' sfu.log | head -1)
  echo "== $bots participants, $cameras cameras, layers ${LAYERS:-h:1500}, focus ${FOCUS:--} other ${OTHER:--}${BWE:+, BWE towards $BWE kbit/s}"
  ./target/release/load --server "http://$ip:3000" --bots "$bots" --cameras "$cameras" \
    --layers "${LAYERS:-h:1500}" ${SLOTS:+--slots $SLOTS} --seconds "${DURATION:-20}" | tail -1
  grep "in room" sfu.log | tail -2
  kill "$sfu_pid"
  wait "$sfu_pid" 2>/dev/null || true
  sleep 1
done
rm -f sfu.log
