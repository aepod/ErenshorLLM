#!/usr/bin/env bash
# generate-training-data.sh
#
# Generates synthetic training data for LoRA fine-tuning by sending diverse
# player messages to every personality via the /v1/respond endpoint.
#
# Output: raw JSONL file with system/user/assistant message triples.
# Run curate-training-data.py afterwards to filter and score.
#
# Usage:
#   ./scripts/generate-training-data.sh [sidecar_url] [output_file]
#
# Requirements:
#   - Sidecar running with LLM enabled
#   - jq installed
#   - curl installed

set -euo pipefail

SIDECAR_URL="${1:-http://127.0.0.1:11435}"
OUTPUT_FILE="${2:-training-data-raw.jsonl}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA_DIR="$SCRIPT_DIR/../data"
PERSONALITIES_DIR="$DATA_DIR/personalities"

# Rate limit: requests per second
RATE_LIMIT=1
DELAY=$(echo "scale=2; 1/$RATE_LIMIT" | bc)

# Diverse player messages covering different dialog categories
MESSAGES=(
    # Greetings
    "Hey there!"
    "Hello, how are you?"
    "What's up?"
    "Hail!"

    # Item questions
    "What armor should I get?"
    "What's the best weapon for my level?"
    "Where can I find good gear?"
    "Is this item worth buying?"
    "What equipment do you use?"

    # Zone questions
    "Where should I level up?"
    "What zone is good for level 20?"
    "Have you been to any dangerous places?"
    "Where's the best hunting spot?"
    "What zone should I avoid?"

    # Combat advice
    "Any tips for fighting bosses?"
    "How do I deal more damage?"
    "What's your combat strategy?"
    "Can you help me with this fight?"
    "How do I survive in dungeons?"

    # Group invites
    "Want to group up?"
    "Looking for a group?"
    "Need another member for the party?"
    "Can I join your group?"

    # Lore queries
    "Tell me about this zone."
    "What do you know about the Sivakayans?"
    "Who are the Followers of Evil?"
    "What's the history of Port Azure?"
    "Have you heard any rumors?"

    # Farewells
    "I have to go, see you later."
    "Good hunting out there."
    "Take care!"

    # Humor
    "Got any jokes?"
    "This game is wild, right?"
    "I just fell off a cliff."

    # Emotional situations
    "I keep dying, this is frustrating."
    "I finally beat that boss!"
    "I'm lost, can you help?"
    "I just got the best drop ever!"

    # Class-specific
    "What spells should I learn?"
    "How do I play my class better?"
    "What's the best build?"

    # Trading
    "Want to trade?"
    "How much is this worth?"
    "Where can I sell this?"

    # General
    "What are you doing here?"
    "What's your favorite thing about Erenshor?"
    "Any quests around here?"
)

# Check prerequisites
if ! command -v jq &>/dev/null; then
    echo "ERROR: jq is required. Install with: apt install jq" >&2
    exit 1
fi

if ! curl -sf "$SIDECAR_URL/health" >/dev/null 2>&1; then
    echo "ERROR: Sidecar not reachable at $SIDECAR_URL" >&2
    echo "Start the sidecar with LLM enabled first." >&2
    exit 1
fi

# Check LLM is enabled
LLM_STATUS=$(curl -sf "$SIDECAR_URL/health" | jq -r '.llm.enabled // false')
if [ "$LLM_STATUS" != "true" ]; then
    echo "WARNING: LLM not enabled. Responses will be template-only." >&2
fi

echo "=== Training Data Generation ==="
echo "Sidecar: $SIDECAR_URL"
echo "Output:  $OUTPUT_FILE"
echo "Messages per personality: ${#MESSAGES[@]}"

# Count personalities
PERSONALITY_COUNT=$(find "$PERSONALITIES_DIR" -name "*.json" 2>/dev/null | wc -l)
echo "Personalities: $PERSONALITY_COUNT"
echo "Expected examples: $((PERSONALITY_COUNT * ${#MESSAGES[@]}))"
echo ""

# Clear output file
> "$OUTPUT_FILE"

TOTAL=0
ERRORS=0

# Zones to rotate through
ZONES=("Port Azure" "Hidden Hills" "Stowaway's Step" "Abyssal Lake" "Braxonian Desert"
       "Elderstone Mines" "Fallen Braxonia" "Loomingwood" "The Blight" "Wickliff")

CLASSES=("Paladin" "Arcanist" "Windblade" "Druid" "Stormcaller" "Reaver")

for personality_file in "$PERSONALITIES_DIR"/*.json; do
    [ -f "$personality_file" ] || continue

    SIM_NAME=$(jq -r '.name // empty' "$personality_file" 2>/dev/null)
    [ -z "$SIM_NAME" ] && continue

    echo "Processing: $SIM_NAME"

    for i in "${!MESSAGES[@]}"; do
        MSG="${MESSAGES[$i]}"

        # Rotate zone and class for variety
        ZONE="${ZONES[$((i % ${#ZONES[@]}))]}"
        PLAYER_CLASS="${CLASSES[$((i % ${#CLASSES[@]}))]}"
        PLAYER_LEVEL=$(( (i % 40) + 1 ))

        # Build request
        REQUEST=$(jq -n \
            --arg msg "$MSG" \
            --arg sim "$SIM_NAME" \
            --arg zone "$ZONE" \
            --arg pclass "$PLAYER_CLASS" \
            --argjson plevel "$PLAYER_LEVEL" \
            '{
                player_message: $msg,
                channel: "say",
                sim_name: $sim,
                zone: $zone,
                relationship: 5.0,
                player_name: "Adventurer",
                player_level: $plevel,
                player_class: $pclass,
                player_guild: "",
                sim_guild: "",
                sim_is_rival: false,
                group_members: []
            }')

        # Send request
        RESPONSE=$(curl -sf -X POST \
            -H "Content-Type: application/json" \
            -d "$REQUEST" \
            "$SIDECAR_URL/v1/respond" 2>/dev/null) || {
            ERRORS=$((ERRORS + 1))
            continue
        }

        # Extract response text and source
        RESP_TEXT=$(echo "$RESPONSE" | jq -r '.response // empty')
        RESP_SOURCE=$(echo "$RESPONSE" | jq -r '.source // "unknown"')

        [ -z "$RESP_TEXT" ] && continue

        # Format as chat-completion training example
        # System prompt is simplified for fine-tuning
        SYSTEM_PROMPT="You are $SIM_NAME, an NPC in the world of Erenshor. Respond in character with 1-2 sentences. Do not use markdown."

        jq -n \
            --arg sys "$SYSTEM_PROMPT" \
            --arg user "$MSG" \
            --arg asst "$RESP_TEXT" \
            --arg source "$RESP_SOURCE" \
            --arg sim "$SIM_NAME" \
            --arg zone "$ZONE" \
            '{
                messages: [
                    {role: "system", content: $sys},
                    {role: "user", content: $user},
                    {role: "assistant", content: $asst}
                ],
                metadata: {
                    source: $source,
                    sim_name: $sim,
                    zone: $zone
                }
            }' >> "$OUTPUT_FILE"

        TOTAL=$((TOTAL + 1))

        # Rate limit
        sleep "$DELAY"
    done
done

echo ""
echo "=== Generation Complete ==="
echo "Total examples: $TOTAL"
echo "Errors: $ERRORS"
echo "Output: $OUTPUT_FILE"
echo ""
echo "Next step: python3 scripts/curate-training-data.py $OUTPUT_FILE"
