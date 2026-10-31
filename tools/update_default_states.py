import argparse
import json
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser(description="Extract default block properties from a Minecraft block report")
    parser.add_argument("report", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    blocks = json.loads(args.report.read_text(encoding="utf-8"))
    defaults = {}
    for name, block in blocks.items():
        states = [state for state in block["states"] if state.get("default")]
        if len(states) != 1:
            raise ValueError(f"Expected one default state for {name}")
        if properties := states[0].get("properties"):
            defaults[name] = properties
    args.output.write_text(
        json.dumps(defaults, sort_keys=True, separators=(",", ":")) + "\n",
        encoding="utf-8",
    )
    print(f"Wrote default properties for {len(defaults)} blocks")


if __name__ == "__main__":
    main()
