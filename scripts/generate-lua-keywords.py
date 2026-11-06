"""Build the Lua keyword catalog from Second Life's SLua definitions.

Input: slua_definitions.yaml's generated viewer keyword file,
https://raw.githubusercontent.com/secondlife/lsl-definitions/refs/heads/main/generated/lua_keywords_pretty.xml
(or a local copy). Output: crates/aurora-viewer/assets/lua/keywords.json,
read by the viewer (keep the two formats in step).

The upstream file is already curated: LuaJIT leftovers (getfenv, setfenv,
loadstring), local-only debug helpers (collectgarbage, graphheap,
graphuserheap) and anything else the simulator does not run are absent, and
the LSL twins live under their `ll.` names next to the Lua libraries, so the
generator only converts the format. Standard library only.

    python scripts/generate-lua-keywords.py [URL-or-path] [assets/lua/keywords.json]

Output, sorted for deterministic diffs:
    {"controls": [...], "types": [...], "events": [...], "functions": [...],
     "constants": [...]}
"""

import argparse
import json
import urllib.request
import xml.etree.ElementTree as ET
from pathlib import Path

DEFAULT_URL = (
    "https://raw.githubusercontent.com/secondlife/lsl-definitions"
    "/refs/heads/main/generated/lua_keywords_pretty.xml"
)
SECTIONS = ("controls", "types", "events", "functions", "constants")


def load(source: str) -> bytes:
    if Path(source).exists():
        return Path(source).read_bytes()
    with urllib.request.urlopen(source) as response:
        return response.read()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", nargs="?", default=DEFAULT_URL)
    parser.add_argument(
        "output",
        nargs="?",
        default="crates/aurora-viewer/assets/lua/keywords.json",
    )
    args = parser.parse_args()
    root = ET.fromstring(load(args.source))
    top = root.find("map")
    assert top is not None, "expected an LLSD map at the top"
    catalog: dict = {}
    for section in SECTIONS:
        words = set()
        for key, section_map in _pairs(top):
            if key == section:
                for word, _ in _pairs(section_map):
                    # The lexer looks identifiers up one by one, so `ll.Say`
                    # and `string.format` contribute their member names; the
                    # dotted forms themselves can never match.
                    words.add(word.split(".")[-1])
        catalog[section] = sorted(words)
    missing = [s for s in SECTIONS if not catalog[s]]
    assert not missing, f"empty sections: {missing}"
    out = Path(args.output)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(catalog, indent=2) + "\n", encoding="utf-8")
    print(f"{out}: {sum(len(v) for v in catalog.values())} words")


def _pairs(map_element):
    children = list(map_element)
    assert len(children) % 2 == 0, "LLSD map with an odd child count"
    for i in range(0, len(children), 2):
        assert children[i].tag == "key", "expected <key> in LLSD map"
        yield children[i].text or "", children[i + 1]


if __name__ == "__main__":
    main()
