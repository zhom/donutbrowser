#!/usr/bin/env python3
"""Copy the first JSON object in a model answer to a file.

    python3 .github/scripts/extract-json.py ANSWER_FILE OUTPUT_FILE

Copilot answers in text. Even when told to return only JSON, a model can wrap
it in a markdown fence or put a sentence before it. This skips anything around
the first object that parses. Exits 0 when it wrote one, 1 when there is none.
"""

import json
import sys


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    with open(sys.argv[1], encoding="utf-8", errors="replace") as fh:
        text = fh.read()
    decoder = json.JSONDecoder()
    start = text.find("{")
    while start != -1:
        try:
            value, _ = decoder.raw_decode(text, start)
        except ValueError:
            start = text.find("{", start + 1)
            continue
        if isinstance(value, dict):
            with open(sys.argv[2], "w", encoding="utf-8") as fh:
                json.dump(value, fh, ensure_ascii=False)
            return 0
        start = text.find("{", start + 1)
    return 1


if __name__ == "__main__":
    sys.exit(main())
