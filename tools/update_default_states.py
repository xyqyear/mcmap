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
    lines = [
        "// Generated from the official Minecraft 26.3 block report by tools/update_default_states.py.",
        "#[rustfmt::skip]",
        "static DEFAULTS: &[(&str, &[(&str, &str)])] = &[",
    ]
    for name, properties in sorted(defaults.items()):
        pairs = ", ".join(
            f"({json.dumps(key)}, {json.dumps(value)})"
            for key, value in sorted(properties.items())
        )
        lines.append(f"    ({json.dumps(name)}, &[{pairs}]),")
    lines.extend([
        "];", "",
        "pub(super) fn properties(name: &str) -> &'static [(&'static str, &'static str)] {",
        "    DEFAULTS",
        "        .binary_search_by_key(&name, |(id, _)| id)",
        "        .map(|index| DEFAULTS[index].1)",
        "        .unwrap_or(&[])",
        "}", "",
    ])
    args.output.write_text("\n".join(lines), encoding="utf-8")
    print(f"Wrote default properties for {len(defaults)} blocks")


if __name__ == "__main__":
    main()
