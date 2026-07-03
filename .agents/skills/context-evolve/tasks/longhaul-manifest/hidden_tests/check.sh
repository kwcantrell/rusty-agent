#!/usr/bin/env bash
# Hidden oracle for longhaul-manifest. Exit 0 == pass. All 20 settings
# delivered one-per-turn across the session must land in manifest.txt as
# '<name> = <value>' lines. Any missing or wrong value fails.
set -euo pipefail
[ -f manifest.txt ]
grep -Eq 'alpha_timeout[[:space:]]*=[[:space:]]*4831([^0-9]|$)' manifest.txt
grep -Eq 'bravo_retries[[:space:]]*=[[:space:]]*7207([^0-9]|$)' manifest.txt
grep -Eq 'charlie_port[[:space:]]*=[[:space:]]*9614([^0-9]|$)' manifest.txt
grep -Eq 'delta_batch[[:space:]]*=[[:space:]]*2358([^0-9]|$)' manifest.txt
grep -Eq 'echo_window[[:space:]]*=[[:space:]]*6079([^0-9]|$)' manifest.txt
grep -Eq 'foxtrot_ttl[[:space:]]*=[[:space:]]*1743([^0-9]|$)' manifest.txt
grep -Eq 'golf_quota[[:space:]]*=[[:space:]]*8926([^0-9]|$)' manifest.txt
grep -Eq 'hotel_depth[[:space:]]*=[[:space:]]*3465([^0-9]|$)' manifest.txt
grep -Eq 'india_limit[[:space:]]*=[[:space:]]*5192([^0-9]|$)' manifest.txt
grep -Eq 'juliet_rate[[:space:]]*=[[:space:]]*7840([^0-9]|$)' manifest.txt
grep -Eq 'kilo_buffer[[:space:]]*=[[:space:]]*2617([^0-9]|$)' manifest.txt
grep -Eq 'lima_threads[[:space:]]*=[[:space:]]*9053([^0-9]|$)' manifest.txt
grep -Eq 'mike_cache[[:space:]]*=[[:space:]]*4278([^0-9]|$)' manifest.txt
grep -Eq 'november_poll[[:space:]]*=[[:space:]]*6531([^0-9]|$)' manifest.txt
grep -Eq 'oscar_burst[[:space:]]*=[[:space:]]*1986([^0-9]|$)' manifest.txt
grep -Eq 'papa_grace[[:space:]]*=[[:space:]]*8342([^0-9]|$)' manifest.txt
grep -Eq 'quebec_span[[:space:]]*=[[:space:]]*3709([^0-9]|$)' manifest.txt
grep -Eq 'romeo_slots[[:space:]]*=[[:space:]]*5864([^0-9]|$)' manifest.txt
grep -Eq 'sierra_fanout[[:space:]]*=[[:space:]]*2145([^0-9]|$)' manifest.txt
grep -Eq 'tango_backoff[[:space:]]*=[[:space:]]*9427([^0-9]|$)' manifest.txt
