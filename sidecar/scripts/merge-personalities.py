#!/usr/bin/env python3
"""
merge-personalities.py

Merges real game data from _full_personality_dump.json into personality JSON files.
Run from the sidecar directory:
    python3 scripts/merge-personalities.py
"""

import json
import glob
import os
import re
import sys


# ── Paths ──────────────────────────────────────────────────────────────────────

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
SIDECAR_DIR = os.path.dirname(SCRIPT_DIR)
PERSONALITY_DIR = os.path.join(SIDECAR_DIR, "data", "personalities")
DUMP_PATH = os.path.join(PERSONALITY_DIR, "_full_personality_dump.json")


# ── Personality type mapping ───────────────────────────────────────────────────
# Game dump uses 0-5. Our JSON uses: 1=Nice, 2=Tryhard, 3=Mean, 5=Neutral.
# Mapping: 0->5(Neutral), 1->1(Nice), 2->2(Tryhard), 3->3(Mean),
#          4->5(Neutral, unknown/default), 5->5(Neutral)

PERSONALITY_TYPE_MAP = {
    0: 5,  # Neutral
    1: 1,  # Nice
    2: 2,  # Tryhard
    3: 3,  # Mean
    4: 5,  # Unknown -> Neutral
    5: 5,  # Neutral
}

PERSONALITY_TYPE_NAMES = {
    1: "Nice",
    2: "Tryhard",
    3: "Mean",
    5: "Neutral",
}


def load_dump():
    """Load the full personality dump file."""
    with open(DUMP_PATH, "r", encoding="utf-8") as f:
        return json.load(f)


def load_personality_files():
    """Load all personality JSON files (excluding _ prefixed files)."""
    pattern = os.path.join(PERSONALITY_DIR, "*.json")
    files = {}
    for path in sorted(glob.glob(pattern)):
        basename = os.path.basename(path)
        if basename.startswith("_"):
            continue
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        files[path] = data
    return files


def build_name_lookup(dump):
    """Build case-insensitive name lookups for actual_sims and sim_tracking."""
    actual_sims = {}
    for name, data in dump.get("actual_sims", {}).items():
        actual_sims[name.lower()] = data

    sim_tracking = {}
    for name, data in dump.get("sim_tracking", {}).items():
        sim_tracking[name.lower()] = data

    return actual_sims, sim_tracking


def map_personality_type(raw_value):
    """Map a raw personality type value from the dump to our schema."""
    return PERSONALITY_TYPE_MAP.get(raw_value, 5)


def update_archetype_level(archetype, level, char_class):
    """
    NOTE: Levels are intentionally NOT written into archetypes.
    All SimPlayers (both premade and generated) level up over time,
    so any level snapshot would go stale. The archetype should describe
    the character's personality/class, not their current level.

    This function is kept as a no-op for documentation purposes.
    """
    return archetype


def merge_actual_sim(personality, actual_data, changes):
    """
    Merge data from actual_sims (28 premade characters with full data).

    Overwrites: chat_modifiers, behavioral_attributes, special_flags,
                personality_type.
    """
    p = actual_data.get("personality", {})
    s = actual_data.get("speech", {})

    # ── personality_type ───────────────────────────────────────────────────
    raw_ptype = p.get("personality_type", 0)
    mapped_ptype = map_personality_type(raw_ptype)
    if personality.get("personality_type") != mapped_ptype:
        old = personality.get("personality_type")
        personality["personality_type"] = mapped_ptype
        old_name = PERSONALITY_TYPE_NAMES.get(old, str(old))
        new_name = PERSONALITY_TYPE_NAMES.get(mapped_ptype, str(mapped_ptype))
        changes.append(f"  personality_type: {old_name} -> {new_name}")

    # ── chat_modifiers ─────────────────────────────────────────────────────
    cm = personality.setdefault("chat_modifiers", {})
    cm_updates = {
        "types_in_all_caps": s.get("types_in_all_caps", False),
        "types_in_all_lowers": s.get("types_in_all_lowers", False),
        "types_in_third_person": s.get("types_in_third_person", False),
        "loves_emojis": s.get("loves_emojis", False),
        "typo_rate": s.get("typo_rate", 0.0),
        "abbreviates": p.get("abbreviates", False),
        "refers_to_self_as": s.get("refers_to_self_as", "") or None,
        "sign_off_lines": s.get("sign_off_lines", []),
    }

    for key, new_val in cm_updates.items():
        # Normalize empty string to None for refers_to_self_as
        if key == "refers_to_self_as" and new_val == "":
            new_val = None
        old_val = cm.get(key)
        if old_val != new_val:
            cm[key] = new_val
            changes.append(f"  chat_modifiers.{key}: {old_val!r} -> {new_val!r}")

    # ── behavioral_attributes ──────────────────────────────────────────────
    ba = personality.setdefault("behavioral_attributes", {})
    ba_updates = {
        "lore_chase": p.get("lore_chase", 0),
        "gear_chase": p.get("gear_chase", 0),
        "social_chase": p.get("social_chase", 0),
        "troublemaker": p.get("troublemaker", 0),
        "dedication_level": p.get("dedication_level", 0),
        "greed": p.get("greed", 1.0),
        "caution": p.get("caution", False),
        "patience": p.get("patience", 1000),
    }

    for key, new_val in ba_updates.items():
        old_val = ba.get(key)
        if old_val != new_val:
            ba[key] = new_val
            changes.append(f"  behavioral_attributes.{key}: {old_val!r} -> {new_val!r}")

    # ── special_flags ──────────────────────────────────────────────────────
    sf = personality.setdefault("special_flags", {})
    sf_updates = {
        "rival": p.get("rival", False),
        "is_gm_character": p.get("is_gm_character", False),
    }

    for key, new_val in sf_updates.items():
        old_val = sf.get(key)
        if old_val != new_val:
            sf[key] = new_val
            changes.append(f"  special_flags.{key}: {old_val!r} -> {new_val!r}")

    return personality


def merge_sim_tracking(personality, tracking_data, changes):
    """
    Merge data from sim_tracking (257 characters with runtime data).

    Updates: personality_type, behavioral_attributes, special_flags,
             guild_affinity.

    NOTE: Does NOT update archetype with level/class for tracking-only chars.
    Non-ActualSims level up over time, so their level is a snapshot that would
    go stale. Only ActualSims (premade prefabs) have fixed levels.
    """
    # ── personality_type ───────────────────────────────────────────────────
    raw_ptype = tracking_data.get("personality", 0)
    mapped_ptype = map_personality_type(raw_ptype)
    if personality.get("personality_type") != mapped_ptype:
        old = personality.get("personality_type")
        personality["personality_type"] = mapped_ptype
        old_name = PERSONALITY_TYPE_NAMES.get(old, str(old))
        new_name = PERSONALITY_TYPE_NAMES.get(mapped_ptype, str(mapped_ptype))
        changes.append(f"  personality_type: {old_name} -> {new_name}")

    # ── behavioral_attributes ──────────────────────────────────────────────
    ba = personality.setdefault("behavioral_attributes", {})
    ba_fields = [
        "lore_chase", "gear_chase", "social_chase",
        "troublemaker", "dedication_level", "greed", "caution",
    ]

    for key in ba_fields:
        new_val = tracking_data.get(key)
        if new_val is not None:
            old_val = ba.get(key)
            if old_val != new_val:
                ba[key] = new_val
                changes.append(
                    f"  behavioral_attributes.{key}: {old_val!r} -> {new_val!r}"
                )

    # ── special_flags ──────────────────────────────────────────────────────
    sf = personality.setdefault("special_flags", {})
    sf_fields = {"rival": False, "is_gm_character": False}

    for key, default in sf_fields.items():
        new_val = tracking_data.get(key, default)
        old_val = sf.get(key)
        if old_val != new_val:
            sf[key] = new_val
            changes.append(f"  special_flags.{key}: {old_val!r} -> {new_val!r}")

    # ── guild_affinity ─────────────────────────────────────────────────────
    guild_id = tracking_data.get("guild_id", "")
    if guild_id:
        new_guild = guild_id
    else:
        new_guild = None

    old_guild = personality.get("guild_affinity")
    if old_guild != new_guild:
        personality["guild_affinity"] = new_guild
        changes.append(f"  guild_affinity: {old_guild!r} -> {new_guild!r}")

    # NOTE: Level/class intentionally NOT updated here.
    # Non-ActualSims level up over time -- their level is save-specific.
    # The archetype string should not contain a stale level snapshot.

    return personality


def remove_style_quirks(personality, changes):
    """Remove the legacy style_quirks field if present (superseded by chat_modifiers)."""
    if "style_quirks" in personality:
        del personality["style_quirks"]
        changes.append("  removed legacy style_quirks field")


def main():
    if not os.path.exists(DUMP_PATH):
        print(f"ERROR: Dump file not found: {DUMP_PATH}")
        sys.exit(1)

    print(f"Loading dump from: {DUMP_PATH}")
    dump = load_dump()

    actual_sims, sim_tracking = build_name_lookup(dump)
    print(f"  actual_sims: {len(actual_sims)} entries")
    print(f"  sim_tracking: {len(sim_tracking)} entries")
    print()

    print(f"Loading personality files from: {PERSONALITY_DIR}")
    personality_files = load_personality_files()
    print(f"  Found {len(personality_files)} personality files")
    print()

    # ── Stats ──────────────────────────────────────────────────────────────
    stats = {
        "total_files": len(personality_files),
        "updated": 0,
        "from_actual_sims": 0,
        "from_tracking_only": 0,
        "no_match": 0,
        "total_changes": 0,
    }

    # ── Process each file ──────────────────────────────────────────────────
    for path, personality in sorted(personality_files.items()):
        name = personality.get("name", "")
        name_lower = name.lower()
        basename = os.path.basename(path)
        changes = []

        has_actual = name_lower in actual_sims
        has_tracking = name_lower in sim_tracking

        if not has_actual and not has_tracking:
            stats["no_match"] += 1
            continue

        # Merge actual_sims data first (more authoritative for premade chars)
        if has_actual:
            merge_actual_sim(personality, actual_sims[name_lower], changes)

        # Then merge sim_tracking data (for all chars including premade)
        # For premade chars, tracking provides level/class/guild that actual_sims lacks
        # For tracking-only chars, this is the sole data source
        if has_tracking:
            # If we already merged actual_sims, only take fields that
            # actual_sims does not provide (level, class, guild_id).
            # For tracking-only chars, take everything.
            if has_actual:
                # actual_sims already set behavioral/special/personality.
                # Only take level/class/guild from tracking.
                tracking_data = sim_tracking[name_lower]
                tracking_changes = []

                # guild_affinity
                guild_id = tracking_data.get("guild_id", "")
                new_guild = guild_id if guild_id else None
                old_guild = personality.get("guild_affinity")
                if old_guild != new_guild:
                    personality["guild_affinity"] = new_guild
                    changes.append(
                        f"  guild_affinity: {old_guild!r} -> {new_guild!r}"
                    )

                # archetype level/class
                level = tracking_data.get("level")
                char_class = tracking_data.get("class", "")
                old_archetype = personality.get("archetype", "")
                new_archetype = update_archetype_level(
                    old_archetype, level, char_class
                )
                if old_archetype != new_archetype:
                    personality["archetype"] = new_archetype
                    changes.append(
                        f"  archetype: \"{old_archetype}\" -> \"{new_archetype}\""
                    )
            else:
                merge_sim_tracking(personality, sim_tracking[name_lower], changes)

        # Clean up legacy field
        remove_style_quirks(personality, changes)

        # ── Write if changed ───────────────────────────────────────────────
        if changes:
            stats["updated"] += 1
            stats["total_changes"] += len(changes)

            if has_actual:
                stats["from_actual_sims"] += 1
            else:
                stats["from_tracking_only"] += 1

            print(f"[UPDATED] {basename} ({name}):")
            for change in changes:
                print(change)
            print()

            with open(path, "w", encoding="utf-8") as f:
                json.dump(personality, f, indent=2, ensure_ascii=False)
                f.write("\n")
        else:
            # Still count the source even if no changes needed
            if has_actual:
                stats["from_actual_sims"] += 1
            elif has_tracking:
                stats["from_tracking_only"] += 1

    # ── Summary ────────────────────────────────────────────────────────────
    print("=" * 60)
    print("MERGE SUMMARY")
    print("=" * 60)
    print(f"  Total personality files:     {stats['total_files']}")
    print(f"  Files updated:               {stats['updated']}")
    print(f"  Total field changes:         {stats['total_changes']}")
    print(f"  Matched from actual_sims:    {stats['from_actual_sims']}")
    print(f"  Matched from tracking only:  {stats['from_tracking_only']}")
    print(f"  No match in dump:            {stats['no_match']}")
    print()

    if stats["no_match"] > 0:
        no_match_names = []
        for path, personality in sorted(personality_files.items()):
            name_lower = personality.get("name", "").lower()
            if name_lower not in actual_sims and name_lower not in sim_tracking:
                no_match_names.append(personality.get("name", ""))
        print(f"  Unmatched files: {', '.join(no_match_names)}")
        print()


if __name__ == "__main__":
    main()
