#!/usr/bin/env python3
"""Extract templates from the built responses.json index back into source JSON files.

Reads the pre-built index (dist/responses.json), groups entries by category,
strips the embedding and per-template category fields, and writes each category
to its own source file matching the RawTemplateFile format:

    {
        "category": "<category_name>",
        "templates": [ { id, text, context_tags, ... }, ... ]
    }
"""

import json
import os
import sys
from collections import defaultdict

DIST_PATH = os.path.join(os.path.dirname(__file__), "dist", "responses.json")
TEMPLATES_DIR = os.path.join(os.path.dirname(__file__), "templates")


def main():
    # Load the built index
    with open(DIST_PATH, "r", encoding="utf-8") as f:
        entries = json.load(f)

    print(f"Loaded {len(entries)} entries from {DIST_PATH}")

    # Group by category
    by_category = defaultdict(list)
    for entry in entries:
        by_category[entry["category"]].append(entry)

    print(f"Found {len(by_category)} categories:")

    os.makedirs(TEMPLATES_DIR, exist_ok=True)

    total_written = 0

    for category in sorted(by_category.keys()):
        templates = by_category[category]

        # Build the RawTemplateFile structure
        raw_templates = []
        for t in templates:
            raw = {
                "id": t["id"],
                "text": t["text"],
                "context_tags": t["context_tags"],
                "zone_affinity": t["zone_affinity"],
                "personality_affinity": t["personality_affinity"],
                "relationship_min": t["relationship_min"],
                "relationship_max": t["relationship_max"],
                "channel": t["channel"],
                "priority": t["priority"],
            }
            raw_templates.append(raw)

        output = {
            "category": category,
            "templates": raw_templates,
        }

        filename = f"{category}.json"
        filepath = os.path.join(TEMPLATES_DIR, filename)

        with open(filepath, "w", encoding="utf-8") as f:
            json.dump(output, f, indent=4, ensure_ascii=False)
            f.write("\n")

        print(f"  {filename}: {len(raw_templates)} templates")
        total_written += len(raw_templates)

    print(f"\nTotal: {total_written} templates written to {len(by_category)} files")

    if total_written != len(entries):
        print(f"WARNING: mismatch! Source had {len(entries)}, wrote {total_written}")
        sys.exit(1)
    else:
        print("Verification passed: all templates accounted for.")


if __name__ == "__main__":
    main()
