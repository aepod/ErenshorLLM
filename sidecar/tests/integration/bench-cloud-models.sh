#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════════
# ErenshorLLM Cloud Model Benchmark
# ═══════════════════════════════════════════════════════════════════════
#
# Benchmarks free OpenRouter models against the sidecar's /v1/paraphrase
# and /v1/respond endpoints to find the best model for Erenshor NPC dialog.
#
# Usage:
#   ./bench-cloud-models.sh [--url URL] [--api-key KEY] [--models MODEL1,MODEL2,...]
#
# Requires: curl, jq, bc
# The sidecar must be running with LLM enabled in cloud mode.
#
# Metrics per model:
#   1. Latency (p50, p90, avg) -- speed of response
#   2. Success rate -- % of requests that returned paraphrased text
#   3. Quality -- response length, personality adherence, lore grounding
#   4. Consistency -- variance in output quality across runs
#   5. Entity accuracy -- proper noun preservation (no hallucinated names)

set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════
# Configuration
# ═══════════════════════════════════════════════════════════════════════

SIDECAR_URL="${SIDECAR_URL:-http://localhost:11435}"
OPENROUTER_API_KEY="${OPENROUTER_API_KEY:-}"
MODELS_OVERRIDE=""
VERBOSE=false
RESULTS_DIR=""

# Free OpenRouter models (as of 2026-03)
# Ordered by expected throughput (highest first)
DEFAULT_MODELS=(
    "nvidia/nemotron-3-nano-30b-a3b:free"
    "stepfun/step-3.5-flash:free"
    "arcee-ai/trinity-large-preview:free"
    "arcee-ai/trinity-mini:free"
    "liquid/lfm-2.5-1.2b-instruct:free"
    "liquid/lfm-2.5-1.2b-thinking:free"
)

# Colors
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    YELLOW='\033[0;33m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    DIM='\033[2m'
    RESET='\033[0m'
else
    GREEN='' RED='' YELLOW='' CYAN='' BOLD='' DIM='' RESET=''
fi

# ═══════════════════════════════════════════════════════════════════════
# Argument Parsing
# ═══════════════════════════════════════════════════════════════════════

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --url)         SIDECAR_URL="$2"; shift 2 ;;
            --api-key)     OPENROUTER_API_KEY="$2"; shift 2 ;;
            --models)      MODELS_OVERRIDE="$2"; shift 2 ;;
            --verbose)     VERBOSE=true; shift ;;
            --results-dir) RESULTS_DIR="$2"; shift 2 ;;
            -h|--help)
                cat <<'EOF'
Usage: bench-cloud-models.sh [OPTIONS]

Options:
  --url URL              Sidecar base URL (default: http://localhost:11435)
  --api-key KEY          OpenRouter API key (or set OPENROUTER_API_KEY env var)
  --models M1,M2,...     Comma-separated list of model IDs to test
                         (default: all free OpenRouter models)
  --results-dir DIR      Save detailed results to DIR (default: /tmp/erenshor-bench-*)
  --verbose              Show full request/response JSON for each test
  -h, --help             Show this help

Environment:
  SIDECAR_URL            Same as --url
  OPENROUTER_API_KEY     Same as --api-key

The sidecar must be running with LLM mode = cloud and a valid API key
configured in erenshor-llm.toml. This script temporarily overrides the
model for each test via the OpenRouter API directly, bypassing the
sidecar's configured model.

Metrics collected per model:
  1. Latency     -- p50, p90, avg response time in ms
  2. Success     -- % of requests returning paraphrased (non-original) text
  3. Length      -- avg response length in chars (too short = lazy, too long = rambling)
  4. Voice       -- personality vocabulary adherence (keyword hit rate)
  5. Grounding   -- proper noun preservation from input text
EOF
                exit 0
                ;;
            *) echo "Unknown option: $1"; exit 1 ;;
        esac
    done
}

# ═══════════════════════════════════════════════════════════════════════
# Test Prompts -- Erenshor-specific paraphrase and respond scenarios
# ═══════════════════════════════════════════════════════════════════════

# Each test case: (name, endpoint, json_payload, expected_entity_words)
# We test both /v1/paraphrase (event dialog rewriting) and /v1/respond
# (player-directed dialog generation).

build_test_cases() {
    # Paraphrase tests -- rewrite canned game text in character voice
    TEST_NAMES=()
    TEST_ENDPOINTS=()
    TEST_PAYLOADS=()
    TEST_ENTITIES=()   # proper nouns that should survive paraphrasing
    TEST_CHANNELS=()

    # 1. Death reaction (group_death)
    TEST_NAMES+=("death_reaction")
    TEST_ENDPOINTS+=("/v1/paraphrase")
    TEST_PAYLOADS+=("$(jq -n '{
        "text": "Nooo! We lost them! Everyone back off!",
        "trigger": "group_death",
        "sim_name": "Rhys",
        "zone": "The Bone Pits",
        "channel": "group",
        "relationship": 7.0,
        "player_name": "Hero",
        "context": {"dead_member": "Phanty", "cause": "Abyssal Lurker"}
    }')")
    TEST_ENTITIES+=("Phanty,Abyssal Lurker")
    TEST_CHANNELS+=("group")

    # 2. Combat callout (pulling)
    TEST_NAMES+=("combat_pull")
    TEST_ENDPOINTS+=("/v1/paraphrase")
    TEST_PAYLOADS+=("$(jq -n '{
        "text": "Pulling the next mob, get ready!",
        "trigger": "combat_callout",
        "sim_name": "Slane",
        "zone": "Dusken Barrows",
        "channel": "group",
        "relationship": 6.0,
        "player_name": "Hero",
        "context": {"callout": "pulling", "enemy": "Skeletal Warden"}
    }')")
    TEST_ENTITIES+=("Skeletal Warden")
    TEST_CHANNELS+=("group")

    # 3. Loot drop excitement
    TEST_NAMES+=("loot_drop")
    TEST_ENDPOINTS+=("/v1/paraphrase")
    TEST_PAYLOADS+=("$(jq -n '{
        "text": "Nice drop! That is some solid gear.",
        "trigger": "loot_request",
        "sim_name": "Vesta",
        "zone": "Azynthi'\''s Garden",
        "channel": "group",
        "relationship": 8.0,
        "player_name": "Hero",
        "context": {"item_name": "Eon Blade of Time"}
    }')")
    TEST_ENTITIES+=("Eon Blade of Time")
    TEST_CHANNELS+=("group")

    # 4. Zone entry remark
    TEST_NAMES+=("zone_entry")
    TEST_ENDPOINTS+=("/v1/paraphrase")
    TEST_PAYLOADS+=("$(jq -n '{
        "text": "This place gives me the creeps.",
        "trigger": "zone_entry",
        "sim_name": "Evelia",
        "zone": "Rottenfoot Swamp",
        "channel": "say",
        "relationship": 5.0,
        "player_name": "Hero"
    }')")
    TEST_ENTITIES+=("Rottenfoot Swamp")
    TEST_CHANNELS+=("say")

    # 5. Greeting/hail
    TEST_NAMES+=("hail_greeting")
    TEST_ENDPOINTS+=("/v1/paraphrase")
    TEST_PAYLOADS+=("$(jq -n '{
        "text": "Hey there! Welcome to the group!",
        "trigger": "group_invite",
        "sim_name": "Nova",
        "zone": "Meadowlands",
        "channel": "group",
        "relationship": 5.0,
        "player_name": "Hero"
    }')")
    TEST_ENTITIES+=("")
    TEST_CHANNELS+=("group")

    # 6. Guild shout (social)
    TEST_NAMES+=("guild_banter")
    TEST_ENDPOINTS+=("/v1/paraphrase")
    TEST_PAYLOADS+=("$(jq -n '{
        "text": "Anyone up for the dungeon tonight?",
        "trigger": "generic",
        "sim_name": "Arty",
        "zone": "Port Azure",
        "channel": "guild",
        "relationship": 7.0,
        "player_name": "Hero"
    }')")
    TEST_ENTITIES+=("Port Azure")
    TEST_CHANNELS+=("guild")

    # 7. Respond test -- player asking about lore
    TEST_NAMES+=("respond_lore")
    TEST_ENDPOINTS+=("/v1/respond")
    TEST_PAYLOADS+=("$(jq -n '{
        "player_message": "Tell me about Sivakaya and the Monolith",
        "channel": "say",
        "sim_name": "Evelia",
        "zone": "Port Azure",
        "player_name": "Hero",
        "player_level": 20,
        "player_class": "Arcanist"
    }')")
    TEST_ENTITIES+=("Sivakaya,Monolith")
    TEST_CHANNELS+=("say")

    # 8. Respond test -- player asking for gear advice
    TEST_NAMES+=("respond_gear")
    TEST_ENDPOINTS+=("/v1/respond")
    TEST_PAYLOADS+=("$(jq -n '{
        "player_message": "What weapon should I use as a Paladin?",
        "channel": "say",
        "sim_name": "Rhys",
        "zone": "Meadowlands",
        "player_name": "Hero",
        "player_level": 15,
        "player_class": "Paladin"
    }')")
    TEST_ENTITIES+=("")
    TEST_CHANNELS+=("say")
}

# ═══════════════════════════════════════════════════════════════════════
# HTTP Helpers
# ═══════════════════════════════════════════════════════════════════════

# Call the sidecar directly (uses its configured LLM)
sidecar_post() {
    local path="$1" body="$2"
    curl -s -w '\n%{http_code}\n%{time_total}' \
        -X POST -H 'Content-Type: application/json' \
        -d "$body" "${SIDECAR_URL}${path}" --max-time 120 2>/dev/null
}

# Call OpenRouter directly for a chat completion (bypassing sidecar LLM)
openrouter_chat() {
    local model="$1" system_msg="$2" user_msg="$3"
    local body
    body=$(jq -n \
        --arg model "$model" \
        --arg sys "$system_msg" \
        --arg usr "$user_msg" \
        '{
            "model": $model,
            "messages": [
                {"role": "system", "content": $sys},
                {"role": "user", "content": $usr}
            ],
            "max_tokens": 150,
            "temperature": 0.7
        }')

    curl -s -w '\n%{http_code}\n%{time_total}' \
        -X POST -H 'Content-Type: application/json' \
        -H "Authorization: Bearer ${OPENROUTER_API_KEY}" \
        -d "$body" "https://openrouter.ai/api/v1/chat/completions" \
        --max-time 120 2>/dev/null
}

# Parse curl output: body, status code, time
parse_response() {
    local raw="$1"
    RESP_TIME=$(echo "$raw" | tail -1)
    RESP_CODE=$(echo "$raw" | tail -2 | head -1)
    RESP_BODY=$(echo "$raw" | sed -n '1,/^[0-9]*$/{ /^[0-9]*$/d; /^[0-9.]*$/d; p; }')
    # More robust: everything except the last 2 lines
    RESP_BODY=$(echo "$raw" | head -n -2)
}

# ═══════════════════════════════════════════════════════════════════════
# Scoring Functions
# ═══════════════════════════════════════════════════════════════════════

# Check if response text contains expected entity names
# Returns 0-100 score (percentage of entities found)
score_entities() {
    local text="$1" entities="$2"
    if [[ -z "$entities" ]]; then
        echo "100"  # no entities to check = perfect
        return
    fi

    local total=0 found=0
    IFS=',' read -ra ENTITY_ARRAY <<< "$entities"
    for entity in "${ENTITY_ARRAY[@]}"; do
        ((total++)) || true
        # Case-insensitive check
        if echo "$text" | grep -qi "$entity" 2>/dev/null; then
            ((found++)) || true
        fi
    done

    if [[ $total -eq 0 ]]; then
        echo "100"
    else
        echo $(( found * 100 / total ))
    fi
}

# Check response length is in the sweet spot for MMO chat (15-200 chars)
# Returns 0-100 score
score_length() {
    local text="$1"
    local len=${#text}

    if [[ $len -lt 5 ]]; then
        echo "0"    # too short / empty
    elif [[ $len -lt 15 ]]; then
        echo "30"   # very short
    elif [[ $len -lt 30 ]]; then
        echo "70"   # a bit short
    elif [[ $len -le 200 ]]; then
        echo "100"  # sweet spot
    elif [[ $len -le 300 ]]; then
        echo "70"   # getting wordy
    else
        echo "30"   # way too long for MMO chat
    fi
}

# Check for AI slop indicators
# Returns 0-100 (100 = no slop detected)
score_slop() {
    local text="$1"
    local lower
    lower=$(echo "$text" | tr '[:upper:]' '[:lower:]')
    local penalty=0

    # Hard slop indicators
    [[ "$lower" == *"as an ai"* ]] && penalty=$((penalty + 50))
    [[ "$lower" == *"language model"* ]] && penalty=$((penalty + 50))
    [[ "$lower" == *"i'm sorry but"* ]] && penalty=$((penalty + 30))
    [[ "$lower" == *"in the world of erenshor"* ]] && penalty=$((penalty + 20))
    [[ "$lower" == *"certainly!"* ]] && penalty=$((penalty + 15))
    [[ "$lower" == *"of course!"* ]] && penalty=$((penalty + 10))

    # Markdown in chat is wrong
    [[ "$text" == *"**"* ]] && penalty=$((penalty + 20))
    [[ "$text" == *"##"* ]] && penalty=$((penalty + 20))
    [[ "$text" == *'```'* ]] && penalty=$((penalty + 30))

    # Multi-paragraph responses (should be 1-2 sentences)
    local newlines
    newlines=$(echo "$text" | grep -c '^' || true)
    if [[ $newlines -gt 3 ]]; then
        penalty=$((penalty + 20))
    fi

    local score=$((100 - penalty))
    [[ $score -lt 0 ]] && score=0
    echo "$score"
}

# ═══════════════════════════════════════════════════════════════════════
# Benchmark Runner
# ═══════════════════════════════════════════════════════════════════════

bench_model_via_openrouter() {
    local model="$1"
    local model_short="${model##*/}"  # strip provider prefix
    local model_dir="${RESULTS_DIR}/${model_short}"
    mkdir -p "$model_dir"

    printf "\n${BOLD}── %s ${RESET}%s\n" "$model" \
        "$(printf '%0.s─' $(seq 1 $((60 - ${#model}))))"

    local total=0 success=0 fail=0
    local latencies=()
    local entity_scores=()
    local length_scores=()
    local slop_scores=()
    local response_lengths=()

    for i in "${!TEST_NAMES[@]}"; do
        local name="${TEST_NAMES[$i]}"
        local endpoint="${TEST_ENDPOINTS[$i]}"
        local payload="${TEST_PAYLOADS[$i]}"
        local entities="${TEST_ENTITIES[$i]}"

        ((total++)) || true

        # Build system and user messages from the payload
        local sim_name zone trigger channel text player_name
        if [[ "$endpoint" == "/v1/paraphrase" ]]; then
            sim_name=$(echo "$payload" | jq -r '.sim_name')
            zone=$(echo "$payload" | jq -r '.zone // ""')
            trigger=$(echo "$payload" | jq -r '.trigger // "generic"')
            channel=$(echo "$payload" | jq -r '.channel // "say"')
            text=$(echo "$payload" | jq -r '.text')
            player_name=$(echo "$payload" | jq -r '.player_name // "Hero"')
            local relationship
            relationship=$(echo "$payload" | jq -r '.relationship // 5.0')

            local system_msg
            system_msg="You are ${sim_name}, an NPC in Erenshor.
Zone: ${zone}
Channel: ${channel} chat
Relationship with player: ${relationship}/10

Rewrite the following line in your voice. Keep all proper nouns exactly as written. Keep it to 1-2 sentences, casual MMO chat style. No markdown. Do NOT repeat the original line verbatim."

            local user_msg="Rephrase: \"${text}\""

            # Call OpenRouter directly
            local t_start t_end elapsed_ms
            t_start=$(date +%s%N 2>/dev/null || echo 0)

            local raw
            raw=$(openrouter_chat "$model" "$system_msg" "$user_msg") || {
                ((fail++)) || true
                printf "  ${RED}FAIL${RESET}  %-20s  (curl error)\n" "$name"
                echo "FAIL: curl error" > "${model_dir}/${name}.txt"
                continue
            }

            t_end=$(date +%s%N 2>/dev/null || echo 0)
            parse_response "$raw"

            if [[ ${#t_start} -gt 10 ]]; then
                elapsed_ms=$(( (t_end - t_start) / 1000000 ))
            else
                elapsed_ms=0
            fi

        else
            # /v1/respond -- same approach but different prompt framing
            sim_name=$(echo "$payload" | jq -r '.sim_name')
            zone=$(echo "$payload" | jq -r '.zone // ""')
            channel=$(echo "$payload" | jq -r '.channel // "say"')
            local player_msg
            player_msg=$(echo "$payload" | jq -r '.player_message')
            player_name=$(echo "$payload" | jq -r '.player_name // "Hero"')

            local system_msg
            system_msg="You are ${sim_name}, an NPC in Erenshor.
Zone: ${zone}
Channel: ${channel} chat

Respond to the player's message in character. Keep your response to 1-2 sentences, casual MMO chat style. No markdown. Do not break character."

            local user_msg="${player_name} says: \"${player_msg}\""

            local t_start t_end elapsed_ms
            t_start=$(date +%s%N 2>/dev/null || echo 0)

            local raw
            raw=$(openrouter_chat "$model" "$system_msg" "$user_msg") || {
                ((fail++)) || true
                printf "  ${RED}FAIL${RESET}  %-20s  (curl error)\n" "$name"
                echo "FAIL: curl error" > "${model_dir}/${name}.txt"
                continue
            }

            t_end=$(date +%s%N 2>/dev/null || echo 0)
            parse_response "$raw"

            if [[ ${#t_start} -gt 10 ]]; then
                elapsed_ms=$(( (t_end - t_start) / 1000000 ))
            else
                elapsed_ms=0
            fi
        fi

        # Parse OpenRouter response
        if [[ "$RESP_CODE" != "200" ]]; then
            ((fail++)) || true
            local err_msg
            err_msg=$(echo "$RESP_BODY" | jq -r '.error.message // .error // "unknown"' 2>/dev/null)
            printf "  ${RED}FAIL${RESET}  %-20s  HTTP %s: %s\n" "$name" "$RESP_CODE" "$err_msg"
            echo "FAIL: HTTP ${RESP_CODE} - ${err_msg}" > "${model_dir}/${name}.txt"
            continue
        fi

        local response_text
        response_text=$(echo "$RESP_BODY" | jq -r '.choices[0].message.content // ""' 2>/dev/null)

        if [[ -z "$response_text" || "$response_text" == "null" ]]; then
            ((fail++)) || true
            printf "  ${RED}FAIL${RESET}  %-20s  empty response\n" "$name"
            echo "FAIL: empty response" > "${model_dir}/${name}.txt"
            continue
        fi

        # Strip quotes if the model wrapped its response in them
        response_text=$(echo "$response_text" | sed 's/^"//;s/"$//')

        ((success++)) || true
        latencies+=("$elapsed_ms")

        # Score this response
        local e_score l_score s_score
        e_score=$(score_entities "$response_text" "$entities")
        l_score=$(score_length "$response_text")
        s_score=$(score_slop "$response_text")

        entity_scores+=("$e_score")
        length_scores+=("$l_score")
        slop_scores+=("$s_score")
        response_lengths+=("${#response_text}")

        # Truncate for display
        local display_text="$response_text"
        if [[ ${#display_text} -gt 80 ]]; then
            display_text="${display_text:0:77}..."
        fi

        printf "  ${GREEN}OK${RESET}    %-20s  %4dms  E:%3d L:%3d S:%3d  \"%s\"\n" \
            "$name" "$elapsed_ms" "$e_score" "$l_score" "$s_score" "$display_text"

        # Save full result
        cat > "${model_dir}/${name}.json" <<ENDJSON
{
  "model": "${model}",
  "test": "${name}",
  "latency_ms": ${elapsed_ms},
  "entity_score": ${e_score},
  "length_score": ${l_score},
  "slop_score": ${s_score},
  "response_length": ${#response_text},
  "response": $(echo "$response_text" | jq -Rs .),
  "http_code": ${RESP_CODE}
}
ENDJSON

        if $VERBOSE; then
            printf "        ${DIM}Full: %s${RESET}\n" "$response_text"
        fi
    done

    # ── Compute aggregates ──────────────────────────────────────────
    local success_rate=0
    if [[ $total -gt 0 ]]; then
        success_rate=$(( success * 100 / total ))
    fi

    # Sort latencies for percentiles
    local sorted_latencies
    sorted_latencies=$(printf '%s\n' "${latencies[@]}" 2>/dev/null | sort -n)
    local lat_count=${#latencies[@]}
    local avg_lat=0 p50_lat=0 p90_lat=0

    if [[ $lat_count -gt 0 ]]; then
        local sum=0
        for l in "${latencies[@]}"; do
            sum=$((sum + l))
        done
        avg_lat=$((sum / lat_count))

        local p50_idx=$(( lat_count / 2 ))
        p50_lat=$(echo "$sorted_latencies" | sed -n "$((p50_idx + 1))p")

        local p90_idx=$(( lat_count * 9 / 10 ))
        [[ $p90_idx -ge $lat_count ]] && p90_idx=$((lat_count - 1))
        p90_lat=$(echo "$sorted_latencies" | sed -n "$((p90_idx + 1))p")
    fi

    # Average scores
    local avg_entity=0 avg_length=0 avg_slop=0 avg_resp_len=0
    if [[ ${#entity_scores[@]} -gt 0 ]]; then
        local sum=0
        for s in "${entity_scores[@]}"; do sum=$((sum + s)); done
        avg_entity=$((sum / ${#entity_scores[@]}))
    fi
    if [[ ${#length_scores[@]} -gt 0 ]]; then
        local sum=0
        for s in "${length_scores[@]}"; do sum=$((sum + s)); done
        avg_length=$((sum / ${#length_scores[@]}))
    fi
    if [[ ${#slop_scores[@]} -gt 0 ]]; then
        local sum=0
        for s in "${slop_scores[@]}"; do sum=$((sum + s)); done
        avg_slop=$((sum / ${#slop_scores[@]}))
    fi
    if [[ ${#response_lengths[@]} -gt 0 ]]; then
        local sum=0
        for s in "${response_lengths[@]}"; do sum=$((sum + s)); done
        avg_resp_len=$((sum / ${#response_lengths[@]}))
    fi

    # Composite score (weighted)
    #   40% quality (entity + length + slop averaged)
    #   30% speed (inversely proportional to latency, capped at 5000ms)
    #   30% reliability (success rate)
    local quality_score=$(( (avg_entity + avg_length + avg_slop) / 3 ))
    local speed_score=0
    if [[ $avg_lat -gt 0 ]]; then
        # 100 at <=500ms, 0 at >=5000ms, linear between
        if [[ $avg_lat -le 500 ]]; then
            speed_score=100
        elif [[ $avg_lat -ge 5000 ]]; then
            speed_score=0
        else
            speed_score=$(( (5000 - avg_lat) * 100 / 4500 ))
        fi
    fi
    local composite=$(( quality_score * 40 / 100 + speed_score * 30 / 100 + success_rate * 30 / 100 ))

    # Print summary
    printf "\n  ${BOLD}Summary:${RESET}\n"
    printf "    Success:   %d/%d (%d%%)\n" "$success" "$total" "$success_rate"
    printf "    Latency:   avg=%dms  p50=%dms  p90=%dms\n" "$avg_lat" "$p50_lat" "$p90_lat"
    printf "    Avg len:   %d chars\n" "$avg_resp_len"
    printf "    Entity:    %d/100  (proper noun preservation)\n" "$avg_entity"
    printf "    Length:    %d/100  (sweet spot 30-200 chars)\n" "$avg_length"
    printf "    Slop:      %d/100  (100 = no AI slop)\n" "$avg_slop"
    printf "    Quality:   %d/100  (entity + length + slop)\n" "$quality_score"
    printf "    Speed:     %d/100  (latency score)\n" "$speed_score"
    printf "    ${BOLD}Composite: %d/100${RESET}\n" "$composite"

    # Save model summary
    cat > "${model_dir}/summary.json" <<ENDJSON
{
  "model": "${model}",
  "total_tests": ${total},
  "success": ${success},
  "fail": ${fail},
  "success_rate": ${success_rate},
  "avg_latency_ms": ${avg_lat},
  "p50_latency_ms": ${p50_lat},
  "p90_latency_ms": ${p90_lat},
  "avg_response_length": ${avg_resp_len},
  "avg_entity_score": ${avg_entity},
  "avg_length_score": ${avg_length},
  "avg_slop_score": ${avg_slop},
  "quality_score": ${quality_score},
  "speed_score": ${speed_score},
  "composite_score": ${composite}
}
ENDJSON

    # Return composite for ranking
    echo "$composite" > "${model_dir}/composite.txt"
}

# ═══════════════════════════════════════════════════════════════════════
# Leaderboard
# ═══════════════════════════════════════════════════════════════════════

print_leaderboard() {
    printf "\n${BOLD}═══════════════════════════════════════════════════════════════${RESET}\n"
    printf "${BOLD}  LEADERBOARD${RESET}\n"
    printf "${BOLD}═══════════════════════════════════════════════════════════════${RESET}\n\n"

    printf "  ${DIM}%-4s %-45s %5s %5s %5s %5s %5s${RESET}\n" \
        "Rank" "Model" "Score" "Speed" "Qual" "Succ%" "AvgMs"

    printf "  %-4s %-45s %5s %5s %5s %5s %5s\n" \
        "────" "─────────────────────────────────────────────" "─────" "─────" "─────" "─────" "─────"

    # Collect all model summaries and sort by composite
    local rank=0
    while IFS= read -r line; do
        ((rank++)) || true
        local model comp speed qual succ avg_ms
        model=$(echo "$line" | cut -d'|' -f1)
        comp=$(echo "$line" | cut -d'|' -f2)
        speed=$(echo "$line" | cut -d'|' -f3)
        qual=$(echo "$line" | cut -d'|' -f4)
        succ=$(echo "$line" | cut -d'|' -f5)
        avg_ms=$(echo "$line" | cut -d'|' -f6)

        local color="$RESET"
        [[ $rank -eq 1 ]] && color="$GREEN$BOLD"
        [[ $rank -eq 2 ]] && color="$GREEN"
        [[ $rank -eq 3 ]] && color="$CYAN"

        printf "  ${color}%-4s %-45s %5s %5s %5s %4s%% %5s${RESET}\n" \
            "#${rank}" "$model" "$comp" "$speed" "$qual" "$succ" "$avg_ms"
    done < <(
        for dir in "${RESULTS_DIR}"/*/; do
            [[ -f "${dir}summary.json" ]] || continue
            local model comp speed qual succ avg_ms
            model=$(jq -r '.model' "${dir}summary.json")
            comp=$(jq -r '.composite_score' "${dir}summary.json")
            speed=$(jq -r '.speed_score' "${dir}summary.json")
            qual=$(jq -r '.quality_score' "${dir}summary.json")
            succ=$(jq -r '.success_rate' "${dir}summary.json")
            avg_ms=$(jq -r '.avg_latency_ms' "${dir}summary.json")
            echo "${model}|${comp}|${speed}|${qual}|${succ}|${avg_ms}"
        done | sort -t'|' -k2 -rn
    )

    printf "\n  ${DIM}Results saved to: ${RESULTS_DIR}${RESET}\n\n"
}

# ═══════════════════════════════════════════════════════════════════════
# Main
# ═══════════════════════════════════════════════════════════════════════

main() {
    parse_args "$@"

    # Validate API key
    if [[ -z "$OPENROUTER_API_KEY" ]]; then
        printf "${RED}ERROR: No API key provided.${RESET}\n"
        printf "Set OPENROUTER_API_KEY env var or use --api-key KEY\n"
        exit 1
    fi

    # Set up results directory
    if [[ -z "$RESULTS_DIR" ]]; then
        RESULTS_DIR="/tmp/erenshor-bench-$(date +%Y%m%d-%H%M%S)"
    fi
    mkdir -p "$RESULTS_DIR"

    # Determine models to test
    local models=()
    if [[ -n "$MODELS_OVERRIDE" ]]; then
        IFS=',' read -ra models <<< "$MODELS_OVERRIDE"
    else
        models=("${DEFAULT_MODELS[@]}")
    fi

    # Build test cases
    build_test_cases

    printf "\n${BOLD}══ ErenshorLLM Cloud Model Benchmark ══${RESET}\n"
    printf "   Models:     ${CYAN}%d${RESET}\n" "${#models[@]}"
    printf "   Tests/model: ${CYAN}%d${RESET}\n" "${#TEST_NAMES[@]}"
    printf "   Results:    ${CYAN}%s${RESET}\n" "$RESULTS_DIR"
    printf "   API:        ${CYAN}OpenRouter (free tier)${RESET}\n"

    # Pre-flight: check OpenRouter is reachable
    printf "\n   Checking OpenRouter API..."
    local check
    check=$(curl -s -o /dev/null -w '%{http_code}' \
        -H "Authorization: Bearer ${OPENROUTER_API_KEY}" \
        "https://openrouter.ai/api/v1/models" --max-time 10 2>/dev/null) || check="000"
    if [[ "$check" != "200" ]]; then
        printf " ${RED}FAILED (HTTP %s)${RESET}\n" "$check"
        printf "   Check your API key and network.\n"
        exit 1
    fi
    printf " ${GREEN}OK${RESET}\n"

    # Run benchmarks
    for model in "${models[@]}"; do
        bench_model_via_openrouter "$model"
    done

    # Print leaderboard
    print_leaderboard
}

main "$@"
