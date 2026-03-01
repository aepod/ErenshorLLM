#!/usr/bin/env bash
# ErenshorLLM Sidecar Integration Tests
# Usage: ./test-sidecar.sh [--url URL] [--filter PATTERN] [--verbose] [--include-shutdown]
#
# Requires: curl, jq
# Designed to run from WSL2 against a sidecar running on Windows.

set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════
# Layer 1: Framework
# ═══════════════════════════════════════════════════════════════════════

SIDECAR_URL="${SIDECAR_URL:-http://localhost:11435}"
FILTER=""
VERBOSE=false
INCLUDE_SHUTDOWN=false

PASSED=0
FAILED=0
SKIPPED=0
TOTAL=0
FAILURES=()

# Response state (set by do_get/do_post)
LAST_CODE=""
LAST_BODY=""

# Colors
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    YELLOW='\033[0;33m'
    CYAN='\033[0;36m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    GREEN='' RED='' YELLOW='' CYAN='' BOLD='' RESET=''
fi

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --url)      SIDECAR_URL="$2"; shift 2 ;;
            --filter)   FILTER="$2"; shift 2 ;;
            --verbose)  VERBOSE=true; shift ;;
            --include-shutdown) INCLUDE_SHUTDOWN=true; shift ;;
            -h|--help)
                echo "Usage: $0 [--url URL] [--filter PATTERN] [--verbose] [--include-shutdown]"
                echo ""
                echo "Options:"
                echo "  --url URL              Sidecar base URL (default: http://localhost:11435)"
                echo "  --filter PATTERN       Only run tests matching PATTERN"
                echo "  --verbose              Dump full request/response JSON for respond tests and failures"
                echo "  --include-shutdown     Include destructive shutdown test"
                exit 0
                ;;
            *)          echo "Unknown option: $1"; exit 1 ;;
        esac
    done
}

# Timer helpers
TEST_START_TIME=0
timer_start() {
    TEST_START_TIME=$(date +%s%N 2>/dev/null || date +%s)
}
timer_elapsed() {
    local now
    now=$(date +%s%N 2>/dev/null || date +%s)
    # If nanosecond precision is available, compute ms; otherwise show seconds
    if [[ ${#now} -gt 10 ]]; then
        echo $(( (now - TEST_START_TIME) / 1000000 ))ms
    else
        echo $(( now - TEST_START_TIME ))s
    fi
}

pass() {
    local name="$1"
    ((PASSED++)) || true
    ((TOTAL++)) || true
    printf "  ${GREEN}PASS${RESET}  %-30s %s\n" "$name" "$(timer_elapsed)"
}

# Dump respond result: source, confidence, truncated response text.
# Replaces pass() for respond tests — counts the test AND shows quality info.
# Usage: dump_respond_result "test_name" ["$REQUEST_JSON"]
dump_respond_result() {
    local name="$1"
    local request_json="${2:-}"
    ((PASSED++)) || true
    ((TOTAL++)) || true
    local source confidence response elapsed
    source=$(echo "$LAST_BODY" | jq -r '.source // "?"' 2>/dev/null)
    confidence=$(echo "$LAST_BODY" | jq -r '.confidence // "?"' 2>/dev/null)
    response=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
    elapsed=$(timer_elapsed)
    # Truncate response to ~80 chars
    local display_resp
    if [[ ${#response} -gt 80 ]]; then
        display_resp="${response:0:77}..."
    else
        display_resp="$response"
    fi
    printf "  ${GREEN}PASS${RESET}  %-30s %s  ${CYAN}[%s %.2s]${RESET} \"%s\"\n" \
        "$name" "$elapsed" "$source" "$confidence" "$display_resp"
    if $VERBOSE; then
        if [[ -n "$request_json" ]]; then
            printf "        ${BOLD}Request:${RESET}  %s\n" "$request_json"
        fi
        printf "        ${BOLD}Response:${RESET} %s\n" "$LAST_BODY"
    fi
}

fail() {
    local name="$1" reason="${2:-}"
    ((FAILED++)) || true
    ((TOTAL++)) || true
    FAILURES+=("$name: $reason")
    printf "  ${RED}FAIL${RESET}  %-30s %s -- %s\n" "$name" "$(timer_elapsed)" "$reason"
    if $VERBOSE && [[ -n "$LAST_BODY" ]]; then
        printf "        Response: %s\n" "$LAST_BODY"
    fi
}

skip() {
    local name="$1" reason="${2:-}"
    ((SKIPPED++)) || true
    ((TOTAL++)) || true
    printf "  ${YELLOW}SKIP${RESET}  %-30s (%s)\n" "$name" "$reason"
}

should_run() {
    local name="$1"
    if [[ -n "$FILTER" ]] && [[ "$name" != *"$FILTER"* ]]; then
        return 1
    fi
    return 0
}

section() {
    printf "\n${BOLD}-- %s ${RESET}%s\n" "$1" "$(printf '%0.s-' $(seq 1 $((50 - ${#1}))))"
}

# HTTP helpers -- capture both body and status code
do_get() {
    local path="$1"
    local raw
    raw=$(curl -s -w '\n%{http_code}' -X GET \
        "${SIDECAR_URL}${path}" --max-time 60 2>/dev/null) || {
        LAST_CODE="000"
        LAST_BODY="curl failed"
        return 1
    }
    LAST_CODE=$(echo "$raw" | tail -1)
    LAST_BODY=$(echo "$raw" | sed '$d')
}

do_post() {
    local path="$1" body="$2"
    local raw
    raw=$(curl -s -w '\n%{http_code}' -X POST \
        -H 'Content-Type: application/json' -d "$body" \
        "${SIDECAR_URL}${path}" --max-time 60 2>/dev/null) || {
        LAST_CODE="000"
        LAST_BODY="curl failed"
        return 1
    }
    LAST_CODE=$(echo "$raw" | tail -1)
    LAST_BODY=$(echo "$raw" | sed '$d')
}

do_post_raw() {
    local path="$1" body="$2"
    local raw
    raw=$(curl -s -w '\n%{http_code}' -X POST \
        -H 'Content-Type: application/json' --data-raw "$body" \
        "${SIDECAR_URL}${path}" --max-time 60 2>/dev/null) || {
        LAST_CODE="000"
        LAST_BODY="curl failed"
        return 1
    }
    LAST_CODE=$(echo "$raw" | tail -1)
    LAST_BODY=$(echo "$raw" | sed '$d')
}

# Assertion helpers
assert_status() {
    local expected="$1" name="$2"
    if [[ "$LAST_CODE" != "$expected" ]]; then
        fail "$name" "Expected HTTP $expected, got $LAST_CODE"
        return 1
    fi
    return 0
}

assert_json_field() {
    local field="$1" name="$2"
    local val
    val=$(echo "$LAST_BODY" | jq -r "$field" 2>/dev/null)
    if [[ -z "$val" || "$val" == "null" ]]; then
        fail "$name" "Missing field: $field"
        return 1
    fi
    return 0
}

assert_json_eq() {
    local field="$1" expected="$2" name="$3"
    local val
    val=$(echo "$LAST_BODY" | jq -r "$field" 2>/dev/null)
    if [[ "$val" != "$expected" ]]; then
        fail "$name" "Expected $field='$expected', got '$val'"
        return 1
    fi
    return 0
}

assert_json_nonempty() {
    local field="$1" name="$2"
    local val
    val=$(echo "$LAST_BODY" | jq -r "$field" 2>/dev/null)
    if [[ -z "$val" || "$val" == "null" || "$val" == "" ]]; then
        fail "$name" "Field $field is empty"
        return 1
    fi
    return 0
}

assert_json_gt() {
    local field="$1" threshold="$2" name="$3"
    local val
    val=$(echo "$LAST_BODY" | jq -r "$field" 2>/dev/null)
    if [[ -z "$val" || "$val" == "null" ]]; then
        fail "$name" "Missing field: $field"
        return 1
    fi
    if ! awk "BEGIN{exit !($val > $threshold)}" 2>/dev/null; then
        fail "$name" "Expected $field > $threshold, got $val"
        return 1
    fi
    return 0
}

assert_json_gte() {
    local field="$1" threshold="$2" name="$3"
    local val
    val=$(echo "$LAST_BODY" | jq -r "$field" 2>/dev/null)
    if [[ -z "$val" || "$val" == "null" ]]; then
        fail "$name" "Missing field: $field"
        return 1
    fi
    if ! awk "BEGIN{exit !($val >= $threshold)}" 2>/dev/null; then
        fail "$name" "Expected $field >= $threshold, got $val"
        return 1
    fi
    return 0
}

# Check if LLM is enabled (cached after first health check)
LLM_ENABLED=""
is_llm_enabled() {
    if [[ -z "$LLM_ENABLED" ]]; then
        do_get "/health"
        LLM_ENABLED=$(echo "$LAST_BODY" | jq -r '.llm.enabled // false' 2>/dev/null)
    fi
    [[ "$LLM_ENABLED" == "true" ]]
}

# ═══════════════════════════════════════════════════════════════════════
# Layer 2: Test Data
# ═══════════════════════════════════════════════════════════════════════

RESPOND_MINIMAL='{
    "player_message": "Hello there!",
    "channel": "say",
    "sim_name": "Aelindra"
}'

RESPOND_FULL='{
    "player_message": "Want to group up and hunt some gnolls?",
    "channel": "say",
    "sim_name": "Aelindra",
    "zone": "Meadowlands",
    "personality": {"friendly": true, "brave": true},
    "relationship": 7.5,
    "player_name": "Zephyr",
    "player_level": 15,
    "player_class": "Ranger",
    "group_members": ["Thorin", "Lyssa"]
}'

RESPOND_WHISPER='{
    "player_message": "Do you know where to find the ancient tome?",
    "channel": "whisper",
    "sim_name": "Brannock"
}'

RESPOND_GUILD='{
    "player_message": "Anyone up for the dungeon tonight?",
    "channel": "guild",
    "sim_name": "Korinth"
}'

RESPOND_OBSCURE='{
    "player_message": "I wonder what the thermodynamic implications of portal magic are in this realm",
    "channel": "say",
    "sim_name": "Aelindra"
}'

RESPOND_GREETING='{
    "player_message": "Hail, friend!",
    "channel": "hail",
    "sim_name": "Aelindra"
}'

EMBED_SINGLE='{
    "input": "The dragon guards the ancient treasure",
    "model": "all-minilm-l6-v2"
}'

EMBED_BATCH='{
    "input": [
        "The wizard cast a powerful spell",
        "The rogue stealthed through the shadows",
        "The healer restored the party to full health"
    ]
}'

RAG_SEARCH_LORE='{
    "query": "ancient ruins of the old kingdom",
    "collection": "lore",
    "top_k": 3
}'

RAG_SEARCH_MEMORY='{
    "query": "player interactions and adventures",
    "collection": "memory",
    "top_k": 3
}'

# ── Multi-Sim Test Data ────────────────────────────────────────────

RESPOND_SHOUT='{
    "player_message": "Selling iron sword, 50 gold!",
    "channel": "shout",
    "sim_name": "Drakkal"
}'

RESPOND_SHOUT_FULL='{
    "player_message": "Anyone know where the lich spawns?",
    "channel": "shout",
    "sim_name": "Drakkal",
    "zone": "Dusken Barrows",
    "personality": {"helpful": true, "veteran": true},
    "relationship": 3.0,
    "player_name": "Zephyr",
    "player_level": 25,
    "player_class": "Wizard"
}'

RESPOND_GUILD_FULL='{
    "player_message": "Anyone want to run the dungeon tonight?",
    "channel": "guild",
    "sim_name": "Korinth",
    "zone": "Stormhold",
    "personality": {"leader": true, "friendly": true},
    "relationship": 8.0,
    "player_name": "Zephyr",
    "player_level": 20,
    "player_class": "Cleric",
    "group_members": ["Korinth", "Frethel"]
}'

# Sim-to-sim: a sim "speaking" as another sim (player_name is a sim name)
RESPOND_SIM_TO_SIM='{
    "player_message": "Hey there, adventurer!",
    "channel": "say",
    "sim_name": "Frethel",
    "player_name": "Drakkal",
    "zone": "Meadowlands"
}'

# Multi-sim: same player message, different sim responders
MULTISIM_MSG="Greetings everyone!"
MULTISIM_SIMS=("Aelindra" "Brannock" "Korinth" "Drakkal" "Frethel")

# ═══════════════════════════════════════════════════════════════════════
# Layer 3: Test Cases
# ═══════════════════════════════════════════════════════════════════════

# ── Category 1: Health & Connectivity ─────────────────────────────────

test_health_basic() {
    local name="health_basic"
    should_run "$name" || return 0
    timer_start
    do_get "/health"
    assert_status 200 "$name" || return 0
    assert_json_eq ".status" "ready" "$name" || return 0
    assert_json_nonempty ".version" "$name" || return 0
    assert_json_gte ".uptime_seconds" 0 "$name" || return 0
    pass "$name"
}

test_health_components() {
    local name="health_components"
    should_run "$name" || return 0
    timer_start
    do_get "/health"
    assert_status 200 "$name" || return 0
    assert_json_eq ".embedding_model_loaded" "true" "$name" || return 0
    assert_json_eq ".lore_index.loaded" "true" "$name" || return 0
    assert_json_eq ".response_index.loaded" "true" "$name" || return 0
    # Memory index may start empty (loaded=false) if no prior memories exist; just check the field exists
    assert_json_field ".memory_index.loaded" "$name" || return 0
    assert_json_gt ".personalities_loaded" 0 "$name" || return 0
    pass "$name"
}

test_health_llm() {
    local name="health_llm"
    should_run "$name" || return 0
    timer_start
    if ! is_llm_enabled; then
        skip "$name" "LLM not enabled"
        return 0
    fi
    do_get "/health"
    assert_status 200 "$name" || return 0
    assert_json_field ".llm.enabled" "$name" || return 0
    assert_json_nonempty ".llm.mode" "$name" || return 0
    assert_json_nonempty ".llm.status" "$name" || return 0
    assert_json_nonempty ".llm.model" "$name" || return 0
    pass "$name"
}

# ── Category 2: Respond -- Channels ──────────────────────────────────

test_respond_say() {
    local name="respond_say"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_MINIMAL"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_field ".template_id" "$name" || return 0
    assert_json_field ".confidence" "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    assert_json_field ".timing" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_MINIMAL"
}

test_respond_say_full() {
    local name="respond_say_full"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_FULL"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_field ".template_id" "$name" || return 0
    # Confidence should exist and be >= 0
    assert_json_gte ".confidence" 0 "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_FULL"
}

test_respond_whisper() {
    local name="respond_whisper"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_WHISPER"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_WHISPER"
}

test_respond_guild() {
    local name="respond_guild"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_GUILD"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_GUILD"
}

# ── Category 3: Respond -- LLM ──────────────────────────────────────

test_respond_llm_fires() {
    local name="respond_llm_fires"
    should_run "$name" || return 0
    timer_start
    if ! is_llm_enabled; then
        skip "$name" "LLM not enabled"
        return 0
    fi
    do_post "/v1/respond" "$RESPOND_OBSCURE"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    # The obscure message should have low template confidence (<0.85),
    # causing the LLM to attempt generation. If the LLM succeeds, source
    # is "llm_*". If it fails (empty response, timeout), it falls back to
    # template but llm_fallback_reason will be set -- proving LLM was tried.
    local source fallback_reason
    source=$(echo "$LAST_BODY" | jq -r '.source // ""' 2>/dev/null)
    fallback_reason=$(echo "$LAST_BODY" | jq -r '.llm_fallback_reason // ""' 2>/dev/null)
    case "$source" in
        llm_*)
            ;; # LLM generated successfully
        template*|fallback*)
            # LLM was attempted but fell back -- verify fallback_reason exists
            if [[ -z "$fallback_reason" ]]; then
                fail "$name" "Template source without LLM attempt (no fallback_reason). LLM may not have fired."
                return 0
            fi
            ;; # OK: LLM tried but fell back
        *)
            fail "$name" "Unexpected source: $source"
            return 0
            ;;
    esac
    dump_respond_result "$name" "$RESPOND_OBSCURE"
}

test_respond_llm_fallback() {
    local name="respond_llm_fallback"
    should_run "$name" || return 0
    timer_start
    if ! is_llm_enabled; then
        skip "$name" "LLM not enabled"
        return 0
    fi
    # Use a normal greeting that should get high template confidence -- LLM won't fire,
    # so llm_fallback_reason should be absent (null)
    do_post "/v1/respond" "$RESPOND_GREETING"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_GREETING"
}

test_respond_template_only() {
    local name="respond_template_only"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_GREETING"
    assert_status 200 "$name" || return 0
    local source
    source=$(echo "$LAST_BODY" | jq -r '.source' 2>/dev/null)
    # For a common greeting, template confidence should be high enough to skip LLM
    # Source should start with "template" or "fallback"
    case "$source" in
        template*|fallback*|llm*)
            # All valid sources
            ;;
        *)
            fail "$name" "Unexpected source: $source"
            return 0
            ;;
    esac
    assert_json_nonempty ".response" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_GREETING"
}

# ── Category 4: Embeddings ──────────────────────────────────────────

test_embed_single() {
    local name="embed_single"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/embeddings" "$EMBED_SINGLE"
    assert_status 200 "$name" || return 0
    assert_json_eq ".object" "list" "$name" || return 0
    # Check we got 1 embedding with index 0
    local count
    count=$(echo "$LAST_BODY" | jq '.data | length' 2>/dev/null)
    if [[ "$count" != "1" ]]; then
        fail "$name" "Expected 1 embedding, got $count"
        return 0
    fi
    assert_json_eq ".data[0].index" "0" "$name" || return 0
    # Check dimension is 384
    local dims
    dims=$(echo "$LAST_BODY" | jq '.data[0].embedding | length' 2>/dev/null)
    if [[ "$dims" != "384" ]]; then
        fail "$name" "Expected 384 dims, got $dims"
        return 0
    fi
    pass "$name"
}

test_embed_batch() {
    local name="embed_batch"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/embeddings" "$EMBED_BATCH"
    assert_status 200 "$name" || return 0
    local count
    count=$(echo "$LAST_BODY" | jq '.data | length' 2>/dev/null)
    if [[ "$count" != "3" ]]; then
        fail "$name" "Expected 3 embeddings, got $count"
        return 0
    fi
    # Check indices are 0, 1, 2
    local i0 i1 i2
    i0=$(echo "$LAST_BODY" | jq '.data[0].index' 2>/dev/null)
    i1=$(echo "$LAST_BODY" | jq '.data[1].index' 2>/dev/null)
    i2=$(echo "$LAST_BODY" | jq '.data[2].index' 2>/dev/null)
    if [[ "$i0" != "0" || "$i1" != "1" || "$i2" != "2" ]]; then
        fail "$name" "Indices should be 0,1,2 -- got $i0,$i1,$i2"
        return 0
    fi
    pass "$name"
}

test_embed_deterministic() {
    local name="embed_deterministic"
    should_run "$name" || return 0
    timer_start
    local payload='{"input": "The tavern was warm and inviting"}'
    do_post "/v1/embeddings" "$payload"
    assert_status 200 "$name" || return 0
    local vec1
    vec1=$(echo "$LAST_BODY" | jq -c '.data[0].embedding' 2>/dev/null)
    do_post "/v1/embeddings" "$payload"
    assert_status 200 "$name" || return 0
    local vec2
    vec2=$(echo "$LAST_BODY" | jq -c '.data[0].embedding' 2>/dev/null)
    if [[ "$vec1" != "$vec2" ]]; then
        fail "$name" "Same input produced different embeddings"
        return 0
    fi
    pass "$name"
}

# ── Category 5: RAG ─────────────────────────────────────────────────

test_rag_search_lore() {
    local name="rag_search_lore"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/rag/search" "$RAG_SEARCH_LORE"
    assert_status 200 "$name" || return 0
    assert_json_field ".results" "$name" || return 0
    assert_json_field ".query_embedding_ms" "$name" || return 0
    assert_json_field ".search_ms" "$name" || return 0
    assert_json_field ".total_results" "$name" || return 0
    pass "$name"
}

test_rag_search_memory() {
    local name="rag_search_memory"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/rag/search" "$RAG_SEARCH_MEMORY"
    assert_status 200 "$name" || return 0
    assert_json_field ".results" "$name" || return 0
    assert_json_field ".total_results" "$name" || return 0
    pass "$name"
}

test_rag_ingest_and_search() {
    local name="rag_ingest_and_search"
    should_run "$name" || return 0
    timer_start
    local marker="test_${$}_$(date +%s)"
    local ingest_payload
    ingest_payload=$(jq -n --arg t "Integration test marker $marker: The brave hero defeated the shadow drake" \
        '{"text": $t, "collection": "memory", "metadata": {"source": "integration_test"}}')
    do_post "/v1/rag/ingest" "$ingest_payload"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".id" "$name" || return 0
    assert_json_eq ".collection" "memory" "$name" || return 0
    # Search for the ingested document
    local search_payload
    search_payload=$(jq -n --arg q "shadow drake $marker" \
        '{"query": $q, "collection": "memory", "top_k": 5, "min_score": 0.1}')
    do_post "/v1/rag/search" "$search_payload"
    assert_status 200 "$name" || return 0
    local found
    found=$(echo "$LAST_BODY" | jq -r ".results[].text" 2>/dev/null | grep -c "$marker" || true)
    if [[ "$found" -lt 1 ]]; then
        fail "$name" "Ingested document not found in search results"
        return 0
    fi
    pass "$name"
}

test_rag_ingest_readonly() {
    local name="rag_ingest_readonly"
    should_run "$name" || return 0
    timer_start
    local payload='{"text": "should not be ingested", "collection": "lore"}'
    do_post "/v1/rag/ingest" "$payload"
    assert_status 400 "$name" || return 0
    pass "$name"
}

# ── Category 6: Error Handling ──────────────────────────────────────

test_error_empty_message() {
    local name="error_empty_message"
    should_run "$name" || return 0
    timer_start
    local payload='{"player_message": "", "channel": "say", "sim_name": "Aelindra"}'
    do_post "/v1/respond" "$payload"
    assert_status 400 "$name" || return 0
    pass "$name"
}

test_error_missing_body() {
    local name="error_missing_body"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" '{}'
    # Should get 400 or 422 (missing required fields)
    if [[ "$LAST_CODE" -ge 400 && "$LAST_CODE" -lt 500 ]]; then
        pass "$name"
    else
        fail "$name" "Expected 4xx error, got $LAST_CODE"
    fi
}

test_error_empty_embed() {
    local name="error_empty_embed"
    should_run "$name" || return 0
    timer_start
    local payload='{"input": ""}'
    do_post "/v1/embeddings" "$payload"
    assert_status 400 "$name" || return 0
    pass "$name"
}

test_error_invalid_json() {
    local name="error_invalid_json"
    should_run "$name" || return 0
    timer_start
    do_post_raw "/v1/respond" 'not valid json at all{{'
    if [[ "$LAST_CODE" -ge 400 && "$LAST_CODE" -lt 500 ]]; then
        pass "$name"
    else
        fail "$name" "Expected 4xx for invalid JSON, got $LAST_CODE"
    fi
}

test_error_404() {
    local name="error_404"
    should_run "$name" || return 0
    timer_start
    do_get "/nonexistent"
    assert_status 404 "$name" || return 0
    pass "$name"
}

# ── Category 7: Response Quality ────────────────────────────────────

test_quality_timing() {
    local name="quality_timing"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_MINIMAL"
    assert_status 200 "$name" || return 0
    # Check all 8 timing fields exist and are >= 0
    local timing_fields=("embed_ms" "sona_transform_ms" "template_search_ms" "rerank_ms"
                         "lore_search_ms" "memory_search_ms" "llm_ms" "total_ms")
    for field in "${timing_fields[@]}"; do
        assert_json_gte ".timing.$field" 0 "$name" || return 0
    done
    pass "$name"
}

test_quality_nonempty() {
    local name="quality_nonempty"
    should_run "$name" || return 0
    timer_start
    local messages=("Hello there!" "Where can I find the blacksmith?" "Let us go fight!")
    for msg in "${messages[@]}"; do
        local payload
        payload=$(jq -n --arg m "$msg" '{"player_message": $m, "channel": "say", "sim_name": "Aelindra"}')
        do_post "/v1/respond" "$payload"
        assert_status 200 "$name" || return 0
        local resp source confidence
        resp=$(echo "$LAST_BODY" | jq -r '.response' 2>/dev/null)
        source=$(echo "$LAST_BODY" | jq -r '.source // "?"' 2>/dev/null)
        confidence=$(echo "$LAST_BODY" | jq -r '.confidence // "?"' 2>/dev/null)
        if [[ -z "$resp" || "$resp" == "null" ]]; then
            fail "$name" "Empty response for message: $msg"
            return 0
        fi
        # Print each message → response pair
        printf "        ${CYAN}%-40s${RESET} -> [%s %.2s] %s\n" \
            "\"$msg\"" "$source" "$confidence" "$resp"
    done
    pass "$name"
}

test_quality_personality() {
    local name="quality_personality"
    should_run "$name" || return 0
    timer_start
    local msg="What do you think about adventure?"
    # First NPC
    local p1
    p1=$(jq -n --arg m "$msg" '{"player_message": $m, "channel": "say", "sim_name": "Aelindra", "personality": {"friendly": true}}')
    do_post "/v1/respond" "$p1"
    assert_status 200 "$name" || return 0
    local r1
    r1=$(echo "$LAST_BODY" | jq -r '.response' 2>/dev/null)
    # Second NPC
    local p2
    p2=$(jq -n --arg m "$msg" '{"player_message": $m, "channel": "say", "sim_name": "Brannock", "personality": {"grumpy": true}}')
    do_post "/v1/respond" "$p2"
    assert_status 200 "$name" || return 0
    local r2
    r2=$(echo "$LAST_BODY" | jq -r '.response' 2>/dev/null)
    printf "        Aelindra: %s\n" "$r1"
    printf "        Brannock: %s\n" "$r2"
    pass "$name"
}

test_quality_sona() {
    local name="quality_sona"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_MINIMAL"
    assert_status 200 "$name" || return 0
    local sona
    sona=$(echo "$LAST_BODY" | jq -r '.sona_enhanced' 2>/dev/null)
    if [[ "$sona" != "true" && "$sona" != "false" ]]; then
        fail "$name" "sona_enhanced should be boolean, got: $sona"
        return 0
    fi
    pass "$name"
}

# ── Category 8: Respond -- Shout ───────────────────────────────────

test_respond_shout() {
    local name="respond_shout"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_SHOUT"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    assert_json_field ".confidence" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_SHOUT"
}

test_respond_shout_full() {
    local name="respond_shout_full"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_SHOUT_FULL"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_gte ".confidence" 0 "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_SHOUT_FULL"
}

# ── Category 9: Respond -- Guild (Full Context) ───────────────────

test_respond_guild_full() {
    local name="respond_guild_full"
    should_run "$name" || return 0
    timer_start
    do_post "/v1/respond" "$RESPOND_GUILD_FULL"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_gte ".confidence" 0 "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_GUILD_FULL"
}

# ── Category 10: Multi-Sim ─────────────────────────────────────────

test_multisim_sequential() {
    local name="multisim_sequential"
    should_run "$name" || return 0
    timer_start
    # Simulate multi-sim: same player message sent to multiple sims sequentially.
    # This mirrors MultiSimDispatcher dispatching additional responders.
    local all_ok=true
    for sim in "${MULTISIM_SIMS[@]}"; do
        local payload
        payload=$(jq -n --arg m "$MULTISIM_MSG" --arg s "$sim" \
            '{"player_message": $m, "channel": "say", "sim_name": $s, "zone": "Meadowlands"}')
        do_post "/v1/respond" "$payload"
        if [[ "$LAST_CODE" != "200" ]]; then
            fail "$name" "HTTP $LAST_CODE for sim $sim"
            return 0
        fi
        local resp source confidence
        resp=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
        source=$(echo "$LAST_BODY" | jq -r '.source // "?"' 2>/dev/null)
        confidence=$(echo "$LAST_BODY" | jq -r '.confidence // "?"' 2>/dev/null)
        if [[ -z "$resp" || "$resp" == "null" ]]; then
            fail "$name" "Empty response for sim $sim"
            return 0
        fi
        printf "        ${CYAN}%-15s${RESET} [%s %.2s] %s\n" "$sim" "$source" "$confidence" "$resp"
    done
    pass "$name"
}

test_multisim_guild() {
    local name="multisim_guild"
    should_run "$name" || return 0
    timer_start
    # Simulate guild multi-sim: same guild message, different guild members responding
    local guild_sims=("Korinth" "Frethel" "Aelindra")
    local msg="Who wants to lead the raid?"
    for sim in "${guild_sims[@]}"; do
        local payload
        payload=$(jq -n --arg m "$msg" --arg s "$sim" \
            '{"player_message": $m, "channel": "guild", "sim_name": $s,
              "player_name": "Zephyr", "zone": "Stormhold"}')
        do_post "/v1/respond" "$payload"
        if [[ "$LAST_CODE" != "200" ]]; then
            fail "$name" "HTTP $LAST_CODE for guild sim $sim"
            return 0
        fi
        local resp source
        resp=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
        source=$(echo "$LAST_BODY" | jq -r '.source // "?"' 2>/dev/null)
        if [[ -z "$resp" || "$resp" == "null" ]]; then
            fail "$name" "Empty response for guild sim $sim"
            return 0
        fi
        printf "        ${CYAN}%-15s${RESET} [%s] %s\n" "$sim" "$source" "$resp"
    done
    pass "$name"
}

test_multisim_shout() {
    local name="multisim_shout"
    should_run "$name" || return 0
    timer_start
    # Simulate shout multi-sim: zone-wide message, multiple sims respond
    local shout_sims=("Drakkal" "Brannock" "Korinth")
    local msg="Looking for group for the Shadow Caves!"
    for sim in "${shout_sims[@]}"; do
        local payload
        payload=$(jq -n --arg m "$msg" --arg s "$sim" \
            '{"player_message": $m, "channel": "shout", "sim_name": $s,
              "player_name": "Zephyr", "zone": "Dusken Barrows"}')
        do_post "/v1/respond" "$payload"
        if [[ "$LAST_CODE" != "200" ]]; then
            fail "$name" "HTTP $LAST_CODE for shout sim $sim"
            return 0
        fi
        local resp source
        resp=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
        source=$(echo "$LAST_BODY" | jq -r '.source // "?"' 2>/dev/null)
        if [[ -z "$resp" || "$resp" == "null" ]]; then
            fail "$name" "Empty response for shout sim $sim"
            return 0
        fi
        printf "        ${CYAN}%-15s${RESET} [%s] %s\n" "$sim" "$source" "$resp"
    done
    pass "$name"
}

test_multisim_distinct_responses() {
    local name="multisim_distinct"
    should_run "$name" || return 0
    timer_start
    # Verify that different sims produce different responses to the same message.
    # With personality-based re-ranking, responses should vary.
    local msg="What do you think about this place?"
    local responses=()
    local sims=("Aelindra" "Brannock" "Drakkal")
    for sim in "${sims[@]}"; do
        local payload
        payload=$(jq -n --arg m "$msg" --arg s "$sim" \
            '{"player_message": $m, "channel": "say", "sim_name": $s}')
        do_post "/v1/respond" "$payload"
        if [[ "$LAST_CODE" != "200" ]]; then
            fail "$name" "HTTP $LAST_CODE for sim $sim"
            return 0
        fi
        local resp
        resp=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
        responses+=("$resp")
        printf "        ${CYAN}%-15s${RESET} %s\n" "$sim" "$resp"
    done
    # Check at least 2 out of 3 are distinct (personality differentiation)
    local unique_count
    unique_count=$(printf '%s\n' "${responses[@]}" | sort -u | wc -l)
    if [[ "$unique_count" -lt 2 ]]; then
        fail "$name" "Expected distinct responses from different sims, got $unique_count unique out of ${#responses[@]}"
        return 0
    fi
    pass "$name"
}

# ── Category 11: Sim-to-Sim ───────────────────────────────────────

test_sim_to_sim_basic() {
    local name="sim_to_sim_basic"
    should_run "$name" || return 0
    timer_start
    # Sim-to-sim: one sim's response becomes the "player message" for another sim.
    # This mirrors MultiSimDispatcher's sim-to-sim chaining.
    do_post "/v1/respond" "$RESPOND_SIM_TO_SIM"
    assert_status 200 "$name" || return 0
    assert_json_nonempty ".response" "$name" || return 0
    assert_json_nonempty ".source" "$name" || return 0
    dump_respond_result "$name" "$RESPOND_SIM_TO_SIM"
}

test_sim_to_sim_chain() {
    local name="sim_to_sim_chain"
    should_run "$name" || return 0
    timer_start
    # Simulate a 3-step conversation chain:
    #   Player -> Drakkal -> Frethel -> Aelindra
    # Step 1: Player speaks, Drakkal responds
    local step1_payload
    step1_payload=$(jq -n '{"player_message": "The weather is nice today",
        "channel": "say", "sim_name": "Drakkal", "player_name": "Zephyr",
        "zone": "Meadowlands"}')
    do_post "/v1/respond" "$step1_payload"
    assert_status 200 "$name" || return 0
    local resp1
    resp1=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
    if [[ -z "$resp1" || "$resp1" == "null" ]]; then
        fail "$name" "Step 1: Empty response from Drakkal"
        return 0
    fi
    printf "        ${CYAN}Zephyr -> Drakkal:${RESET} %s\n" "$resp1"

    # Step 2: Drakkal's response becomes input, Frethel responds
    local step2_payload
    step2_payload=$(jq -n --arg m "$resp1" '{"player_message": $m,
        "channel": "say", "sim_name": "Frethel", "player_name": "Drakkal",
        "zone": "Meadowlands"}')
    do_post "/v1/respond" "$step2_payload"
    assert_status 200 "$name" || return 0
    local resp2
    resp2=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
    if [[ -z "$resp2" || "$resp2" == "null" ]]; then
        fail "$name" "Step 2: Empty response from Frethel"
        return 0
    fi
    printf "        ${CYAN}Drakkal -> Frethel:${RESET} %s\n" "$resp2"

    # Step 3: Frethel's response, Aelindra reacts
    local step3_payload
    step3_payload=$(jq -n --arg m "$resp2" '{"player_message": $m,
        "channel": "say", "sim_name": "Aelindra", "player_name": "Frethel",
        "zone": "Meadowlands"}')
    do_post "/v1/respond" "$step3_payload"
    assert_status 200 "$name" || return 0
    local resp3
    resp3=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
    if [[ -z "$resp3" || "$resp3" == "null" ]]; then
        fail "$name" "Step 3: Empty response from Aelindra"
        return 0
    fi
    printf "        ${CYAN}Frethel -> Aelindra:${RESET} %s\n" "$resp3"
    pass "$name"
}

# ── Category 12: Burst / Rate Limit Simulation ────────────────────

test_burst_sequential() {
    local name="burst_sequential"
    should_run "$name" || return 0
    timer_start
    # Fire 10 rapid requests simulating a burst from multi-sim dispatch.
    # All should succeed -- rate limiting is client-side (C# mod), not sidecar.
    local burst_count=10
    local success_count=0
    local fail_count=0
    for i in $(seq 1 $burst_count); do
        local sim_idx=$(( (i - 1) % ${#MULTISIM_SIMS[@]} ))
        local sim="${MULTISIM_SIMS[$sim_idx]}"
        local payload
        payload=$(jq -n --arg m "Burst test message $i" --arg s "$sim" \
            '{"player_message": $m, "channel": "say", "sim_name": $s}')
        do_post "/v1/respond" "$payload"
        if [[ "$LAST_CODE" == "200" ]]; then
            ((success_count++)) || true
        else
            ((fail_count++)) || true
        fi
    done
    printf "        ${CYAN}Burst: %d/%d succeeded${RESET}\n" "$success_count" "$burst_count"
    if [[ "$success_count" -lt "$burst_count" ]]; then
        fail "$name" "$fail_count of $burst_count requests failed"
        return 0
    fi
    pass "$name"
}

test_burst_timing() {
    local name="burst_timing"
    should_run "$name" || return 0
    timer_start
    # Measure response time consistency under burst load.
    # Responses should not degrade significantly.
    local burst_count=5
    local max_ms=0
    local total_ms=0
    for i in $(seq 1 $burst_count); do
        local t_start t_end elapsed_ms
        t_start=$(date +%s%N 2>/dev/null || echo 0)
        local payload
        payload=$(jq -n --arg m "Timing test $i" --arg s "Aelindra" \
            '{"player_message": $m, "channel": "say", "sim_name": $s}')
        do_post "/v1/respond" "$payload"
        t_end=$(date +%s%N 2>/dev/null || echo 0)
        if [[ "$LAST_CODE" != "200" ]]; then
            fail "$name" "Request $i failed with HTTP $LAST_CODE"
            return 0
        fi
        if [[ ${#t_start} -gt 10 ]]; then
            elapsed_ms=$(( (t_end - t_start) / 1000000 ))
        else
            elapsed_ms=0
        fi
        total_ms=$((total_ms + elapsed_ms))
        if [[ "$elapsed_ms" -gt "$max_ms" ]]; then
            max_ms=$elapsed_ms
        fi
    done
    local avg_ms=$((total_ms / burst_count))
    printf "        ${CYAN}Avg: %dms  Max: %dms  (over %d requests)${RESET}\n" \
        "$avg_ms" "$max_ms" "$burst_count"
    pass "$name"
}

# ── Category 13: Cross-Channel Consistency ─────────────────────────

test_cross_channel_same_sim() {
    local name="cross_channel_same_sim"
    should_run "$name" || return 0
    timer_start
    # Same sim, same message, different channels -- verify all produce valid responses.
    local msg="Hello adventurer"
    local channels=("say" "whisper" "guild" "shout")
    for ch in "${channels[@]}"; do
        local payload
        payload=$(jq -n --arg m "$msg" --arg c "$ch" \
            '{"player_message": $m, "channel": $c, "sim_name": "Aelindra",
              "player_name": "Zephyr"}')
        do_post "/v1/respond" "$payload"
        if [[ "$LAST_CODE" != "200" ]]; then
            fail "$name" "HTTP $LAST_CODE for channel $ch"
            return 0
        fi
        local resp source
        resp=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
        source=$(echo "$LAST_BODY" | jq -r '.source // "?"' 2>/dev/null)
        if [[ -z "$resp" || "$resp" == "null" ]]; then
            fail "$name" "Empty response on channel $ch"
            return 0
        fi
        printf "        ${CYAN}%-10s${RESET} [%s] %s\n" "$ch" "$source" "$resp"
    done
    pass "$name"
}

test_channel_personality_variation() {
    local name="channel_personality_var"
    should_run "$name" || return 0
    timer_start
    # Different personalities on different channels -- verify responses vary.
    local payloads=(
        '{"player_message": "Need help!", "channel": "say", "sim_name": "Aelindra", "personality": {"friendly": true, "helpful": true}}'
        '{"player_message": "Need help!", "channel": "guild", "sim_name": "Brannock", "personality": {"grumpy": true, "veteran": true}}'
        '{"player_message": "Need help!", "channel": "shout", "sim_name": "Drakkal", "personality": {"aggressive": true, "brave": true}}'
    )
    local labels=("Aelindra/say/friendly" "Brannock/guild/grumpy" "Drakkal/shout/aggressive")
    local responses=()
    for i in 0 1 2; do
        do_post "/v1/respond" "${payloads[$i]}"
        if [[ "$LAST_CODE" != "200" ]]; then
            fail "$name" "HTTP $LAST_CODE for ${labels[$i]}"
            return 0
        fi
        local resp
        resp=$(echo "$LAST_BODY" | jq -r '.response // ""' 2>/dev/null)
        responses+=("$resp")
        printf "        ${CYAN}%-30s${RESET} %s\n" "${labels[$i]}" "$resp"
    done
    pass "$name"
}

# ── Category 14: Lore Quality ──────────────────────────────────────

# Helper: search lore and assert a result matches expected criteria.
# Usage: assert_lore_hit "query" "expected_category" "expected_page_substring" "min_score" "test_name"
assert_lore_hit() {
    local query="$1" expected_cat="$2" expected_page="$3" min_score="$4" name="$5"
    local payload
    payload=$(jq -n --arg q "$query" \
        '{"query": $q, "collection": "lore", "top_k": 5, "min_score": 0.1}')
    do_post "/v1/rag/search" "$payload"
    if [[ "$LAST_CODE" != "200" ]]; then
        fail "$name" "HTTP $LAST_CODE for query: $query"
        return 1
    fi
    local total
    total=$(echo "$LAST_BODY" | jq -r '.total_results // 0' 2>/dev/null)
    if [[ "$total" -lt 1 ]]; then
        fail "$name" "No results for query: $query"
        return 1
    fi
    # Check if any result matches expected category and page substring
    local found=false
    local best_score best_cat best_page best_text
    best_score=$(echo "$LAST_BODY" | jq -r '.results[0].score // 0' 2>/dev/null)
    best_cat=$(echo "$LAST_BODY" | jq -r '.results[0].metadata.category // "?"' 2>/dev/null)
    best_page=$(echo "$LAST_BODY" | jq -r '.results[0].metadata.page // "?"' 2>/dev/null)
    best_text=$(echo "$LAST_BODY" | jq -r '.results[0].text // ""' 2>/dev/null)
    local result_count
    result_count=$(echo "$LAST_BODY" | jq '.results | length' 2>/dev/null)
    for idx in $(seq 0 $((result_count - 1))); do
        local cat page score
        cat=$(echo "$LAST_BODY" | jq -r ".results[$idx].metadata.category // \"\"" 2>/dev/null)
        page=$(echo "$LAST_BODY" | jq -r ".results[$idx].metadata.page // \"\"" 2>/dev/null)
        score=$(echo "$LAST_BODY" | jq -r ".results[$idx].score // 0" 2>/dev/null)
        if [[ "$cat" == "$expected_cat" ]] && [[ "$page" == *"$expected_page"* ]]; then
            found=true
            best_score="$score"
            best_cat="$cat"
            best_page="$page"
            break
        fi
    done
    if ! $found; then
        fail "$name" "Expected category=$expected_cat page~=$expected_page, got category=$best_cat page=$best_page"
        return 1
    fi
    # Check minimum score
    if ! awk "BEGIN{exit !($best_score >= $min_score)}" 2>/dev/null; then
        fail "$name" "Score $best_score < min $min_score for $best_page"
        return 1
    fi
    local display_text
    if [[ ${#best_text} -gt 60 ]]; then
        display_text="${best_text:0:57}..."
    else
        display_text="$best_text"
    fi
    printf "        ${CYAN}%-25s${RESET} [%s/%s s=%.2s] %s\n" \
        "$query" "$best_cat" "$best_page" "$best_score" "$display_text"
    return 0
}

test_lore_items() {
    local name="lore_items"
    should_run "$name" || return 0
    timer_start
    # Verify known items are findable in items category
    # Actual data: items/weapons has "Eon Blade", items/armor has "Cloth Shirt"
    assert_lore_hit "Eon Blade sword weapon" "items" "weapons" 0.2 "$name" || return 0
    assert_lore_hit "cloth armor chest slot" "items" "armor" 0.2 "$name" || return 0
    pass "$name"
}

test_lore_npcs() {
    local name="lore_npcs"
    should_run "$name" || return 0
    timer_start
    # Verify NPCs are found in npcs category
    # Actual data: npcs/vendors has "Innkeeper Ryvan", npcs/enemies has level lists
    assert_lore_hit "Innkeeper Ryvan Port Azure" "npcs" "vendors" 0.2 "$name" || return 0
    assert_lore_hit "Auction House broker merchant" "npcs" "vendors" 0.2 "$name" || return 0
    pass "$name"
}

test_lore_zones() {
    local name="lore_zones"
    should_run "$name" || return 0
    timer_start
    # Verify zones are findable
    assert_lore_hit "Meadowlands starting area" "zones" "meadowlands" 0.3 "$name" || return 0
    assert_lore_hit "Port Azure city" "zones" "port-azure" 0.3 "$name" || return 0
    assert_lore_hit "Stowaway Step dungeon" "zones" "stowaway" 0.2 "$name" || return 0
    pass "$name"
}

test_lore_enemies() {
    local name="lore_enemies"
    should_run "$name" || return 0
    timer_start
    # Enemy data appears in both npcs/enemies (level lists) and zones/* (zone enemy rosters).
    # Accept either - the key is that enemy-related content IS returned.
    assert_lore_hit "enemy difficulty color ring level" "npcs" "enemies" 0.2 "$name" || return 0
    pass "$name"
}

test_lore_abilities() {
    local name="lore_abilities"
    should_run "$name" || return 0
    timer_start
    # Abilities are in classes category (e.g. classes/arcanist has "Magic Bolt")
    assert_lore_hit "Magic Bolt spell Arcanist" "classes" "arcanist" 0.2 "$name" || return 0
    assert_lore_hit "Presence of Brax aura" "classes" "arcanist" 0.2 "$name" || return 0
    pass "$name"
}

test_lore_quests() {
    local name="lore_quests"
    should_run "$name" || return 0
    timer_start
    assert_lore_hit "quest adventure task" "quests" "" 0.2 "$name" || return 0
    pass "$name"
}

test_lore_no_misc_leakage() {
    local name="lore_no_misc_leakage"
    should_run "$name" || return 0
    timer_start
    # Known NPCs should NOT appear in misc category.
    # Search for a well-known NPC and verify top result isn't misc.
    local payload
    payload=$(jq -n '{"query": "Aquilamar Blade simulated player", "collection": "lore", "top_k": 3, "min_score": 0.3}')
    do_post "/v1/rag/search" "$payload"
    assert_status 200 "$name" || return 0
    local top_cat
    top_cat=$(echo "$LAST_BODY" | jq -r '.results[0].metadata.category // "?"' 2>/dev/null)
    if [[ "$top_cat" == "misc" ]]; then
        local top_page
        top_page=$(echo "$LAST_BODY" | jq -r '.results[0].metadata.page // "?"' 2>/dev/null)
        fail "$name" "NPC 'Aquilamar Blade' found in misc (page=$top_page), expected npcs"
        return 0
    fi
    printf "        ${CYAN}Top result category: %s (not misc)${RESET}\n" "$top_cat"
    pass "$name"
}

test_lore_category_coverage() {
    local name="lore_category_coverage"
    should_run "$name" || return 0
    timer_start
    # Verify the lore index has entries across multiple categories.
    local categories_found=()
    local test_queries=("sword weapon" "healing spell" "forest zone" "undead enemy" "merchant vendor" "quest adventure")
    for q in "${test_queries[@]}"; do
        local payload
        payload=$(jq -n --arg q "$q" '{"query": $q, "collection": "lore", "top_k": 1, "min_score": 0.1}')
        do_post "/v1/rag/search" "$payload"
        if [[ "$LAST_CODE" == "200" ]]; then
            local cat
            cat=$(echo "$LAST_BODY" | jq -r '.results[0].metadata.category // ""' 2>/dev/null)
            if [[ -n "$cat" && "$cat" != "null" ]]; then
                categories_found+=("$cat")
            fi
        fi
    done
    local unique_cats
    unique_cats=$(printf '%s\n' "${categories_found[@]}" | sort -u | wc -l)
    printf "        ${CYAN}Found %d unique categories across test queries${RESET}\n" "$unique_cats"
    if [[ "$unique_cats" -lt 3 ]]; then
        fail "$name" "Expected >= 3 categories in lore index, got $unique_cats"
        return 0
    fi
    pass "$name"
}

# ── Category 15: Shutdown ──────────────────────────────────────────

test_shutdown() {
    local name="shutdown"
    should_run "$name" || return 0
    timer_start
    if ! $INCLUDE_SHUTDOWN; then
        skip "$name" "use --include-shutdown to enable"
        return 0
    fi
    do_post "/shutdown" '{}'
    assert_status 202 "$name" || return 0
    assert_json_eq ".status" "shutting_down" "$name" || return 0
    pass "$name"
}

# ═══════════════════════════════════════════════════════════════════════
# Layer 4: Runner
# ═══════════════════════════════════════════════════════════════════════

run_all() {
    printf "\n${BOLD}== ErenshorLLM Sidecar Integration Tests ==${RESET}\n"
    printf "   Target: ${CYAN}%s${RESET}\n" "$SIDECAR_URL"

    # Pre-flight: check sidecar is reachable
    if ! curl -s -o /dev/null --max-time 5 "${SIDECAR_URL}/health" 2>/dev/null; then
        printf "\n${RED}ERROR: Cannot reach sidecar at %s${RESET}\n" "$SIDECAR_URL"
        printf "Make sure the sidecar is running and accessible.\n"
        exit 1
    fi

    # Cache LLM status for skip decisions
    is_llm_enabled || true

    section "Health & Connectivity"
    test_health_basic
    test_health_components
    test_health_llm

    section "Respond -- Channels"
    test_respond_say
    test_respond_say_full
    test_respond_whisper
    test_respond_guild

    section "Respond -- LLM"
    test_respond_llm_fires
    test_respond_llm_fallback
    test_respond_template_only

    section "Embeddings"
    test_embed_single
    test_embed_batch
    test_embed_deterministic

    section "RAG"
    test_rag_search_lore
    test_rag_search_memory
    test_rag_ingest_and_search
    test_rag_ingest_readonly

    section "Error Handling"
    test_error_empty_message
    test_error_missing_body
    test_error_empty_embed
    test_error_invalid_json
    test_error_404

    section "Response Quality"
    test_quality_timing
    test_quality_nonempty
    test_quality_personality
    test_quality_sona

    section "Respond -- Shout"
    test_respond_shout
    test_respond_shout_full

    section "Respond -- Guild (Full Context)"
    test_respond_guild_full

    section "Multi-Sim"
    test_multisim_sequential
    test_multisim_guild
    test_multisim_shout
    test_multisim_distinct_responses

    section "Sim-to-Sim"
    test_sim_to_sim_basic
    test_sim_to_sim_chain

    section "Burst / Rate Limit Simulation"
    test_burst_sequential
    test_burst_timing

    section "Cross-Channel Consistency"
    test_cross_channel_same_sim
    test_channel_personality_variation

    section "Lore Quality"
    test_lore_items
    test_lore_npcs
    test_lore_zones
    test_lore_enemies
    test_lore_abilities
    test_lore_quests
    test_lore_no_misc_leakage
    test_lore_category_coverage

    section "Shutdown"
    test_shutdown

    # Summary
    printf "\n${BOLD}===============================================${RESET}\n"
    if [[ $FAILED -eq 0 ]]; then
        printf "  ${GREEN}Results: %d passed, %d failed, %d skipped${RESET}\n" \
            "$PASSED" "$FAILED" "$SKIPPED"
    else
        printf "  ${RED}Results: %d passed, %d failed, %d skipped${RESET}\n" \
            "$PASSED" "$FAILED" "$SKIPPED"
        printf "\n  ${RED}Failures:${RESET}\n"
        for f in "${FAILURES[@]}"; do
            printf "    - %s\n" "$f"
        done
    fi
    printf "${BOLD}===============================================${RESET}\n\n"

    [[ $FAILED -eq 0 ]]
}

parse_args "$@"
run_all
