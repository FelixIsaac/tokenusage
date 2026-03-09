#!/bin/bash
# Recording script: simulates typing tu commands for asciinema

type_text() {
  for (( i=0; i<${#1}; i++ )); do
    printf '%s' "${1:$i:1}"
    sleep 0.1
  done
}

PROMPT='\033[1;32m❯\033[0m '

# --- Command 1: tu ---
sleep 0.4
printf "$PROMPT"
sleep 0.2
type_text "tu"
sleep 0.35
printf '\n'

tu 2>&1
sleep 1.5

# Scroll to top: clear and re-display top portion
clear
printf "${PROMPT}\033[2mtu\033[0m\n"
tu 2>&1 | head -n 42
sleep 2.5

# --- Command 2: tu --since 2026-03-01 ---
clear
sleep 0.3
printf "$PROMPT"
sleep 0.2
type_text "tu --since 2026-03-01"
sleep 0.35
printf '\n'

tu --since 2026-03-01 2>&1
sleep 2.5
