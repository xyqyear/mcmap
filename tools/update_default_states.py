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
        defaults[name] = states[0].get("properties", {})
    lines = [
        "// Generated from the official Minecraft 26.3 block report by tools/update_default_states.py.",
        "pub(super) struct DefaultState {",
        "    pub name: &'static str,",
        "    pub encoded: &'static str,",
        "    pub properties: &'static [(&'static str, &'static str)],",
        "    pub snowy: bool,",
        "}",
        "",
        "#[rustfmt::skip]",
        "pub(super) static DEFAULTS: &[DefaultState] = &[",
    ]
    for name, properties in sorted(defaults.items()):
        pairs = ", ".join(
            f"({json.dumps(key)}, {json.dumps(value)})"
            for key, value in sorted(properties.items())
        )
        encoded = name + "|" + ",".join(
            f"{key}={value}" for key, value in sorted(properties.items())
            if key not in {"waterlogged", "powered"}
        )
        snowy = "true" if properties.get("snowy") == "true" else "false"
        lines.append(
            f"    DefaultState {{ name: {json.dumps(name)}, encoded: {json.dumps(encoded)}, "
            f"properties: &[{pairs}], snowy: {snowy} }},"
        )
    lines.extend([
        "];", "",
        "static BY_NAME: once_cell::sync::Lazy<ahash::AHashMap<&'static str, &'static DefaultState>> =",
        "    once_cell::sync::Lazy::new(|| DEFAULTS.iter().map(|state| (state.name, state)).collect());",
        "",
        "pub(super) fn state(name: &str) -> Option<&'static DefaultState> {",
        "    BY_NAME.get(name).copied()",
        "}", "",
    ])
    args.output.write_text("\n".join(lines), encoding="utf-8")
    print(f"Wrote default properties for {len(defaults)} blocks")


if __name__ == "__main__":
    main()
