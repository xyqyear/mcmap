"""Server flavor definitions: Forge legacy variants and modern vanilla.

mcmap supports three world types: Forge 1.7.10 with NotEnoughIDs (NEID),
Forge 1.12.2 with RoughlyEnoughIDs (REI), and 1.13+ vanilla. Pure vanilla
1.7.10 / 1.12.2 are not supported because gen-palette legacy/forge112 reads
the FML block registry from level.dat, which only Forge writes.

The "latest" flavor's mc_version is resolved from Mojang's piston-meta at
session start (see cache.resolve_latest_release). All other versions are
pinned. Java major versions for legacy MC are pinned at 8; for "latest" the
manifest's javaVersion.majorVersion field is the source of truth and is
filled in lazily.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class ModSpec:
    """A mod jar to drop into mods/ before boot."""
    key: str          # cache slug (e.g. "notenoughids")
    filename: str     # final on-disk name; the cache module decides the URL


@dataclass
class Flavor:
    id: str                       # slug used in test ids and dirs
    distribution: str             # "vanilla" or "forge"
    mc_version: str               # "1.7.10", "1.12.2", or "" for latest (resolved)
    forge_version: str = ""       # full forge version string, e.g. "10.13.4.1614-1.7.10"
    java_major: int = 8           # 0 means "follow manifest" (used for latest)
    mods: list[ModSpec] = field(default_factory=list)
    level_type: str = "FLAT"      # FLAT for legacy, "minecraft:flat" for modern
    generator_settings: str = ""  # legacy semicolon format; empty for modern
    palette_format: str = "modern"  # "modern" / "legacy" / "forge112" — picks gen-palette subcommand


def build_flavors() -> list[Flavor]:
    """Build the static flavor list. The 'latest' flavor's mc_version is filled
    in by cache.resolve_latest_release at session start.
    """
    return [
        Flavor(
            id="forge-1.7.10-neid",
            distribution="forge",
            mc_version="1.7.10",
            forge_version="10.13.4.1614-1.7.10",
            java_major=8,
            mods=[ModSpec(key="notenoughids-1.7.10", filename="NotEnoughIDs.jar")],
            level_type="FLAT",
            generator_settings="3;7,2*3,2;1",
            palette_format="legacy",
        ),
        Flavor(
            id="forge-1.12.2-rei",
            distribution="forge",
            mc_version="1.12.2",
            forge_version="14.23.5.2859",
            java_major=8,
            mods=[
                ModSpec(key="rei-1.12.2", filename="RoughlyEnoughIDs.jar"),
                ModSpec(key="mixinbooter-1.12.2", filename="MixinBooter.jar"),
            ],
            level_type="FLAT",
            generator_settings="3;7,2*3,2;1",
            palette_format="forge112",
        ),
        Flavor(
            id="vanilla-latest",
            distribution="vanilla",
            mc_version="",        # filled in at session start
            java_major=0,         # filled in at session start
            level_type="minecraft:flat",
            generator_settings="",
            palette_format="modern",
        ),
    ]


# --- Convenience lookups ---------------------------------------------------


_FLAVORS_CACHE: list[Flavor] | None = None


def all_flavors() -> list[Flavor]:
    """Return the (cached) flavor list. Mutations made by cache.resolve_latest
    persist on the cached instance, so subsequent callers see the resolved
    fields.
    """
    global _FLAVORS_CACHE
    if _FLAVORS_CACHE is None:
        _FLAVORS_CACHE = build_flavors()
    return _FLAVORS_CACHE


def by_id(flavor_id: str) -> Flavor:
    for f in all_flavors():
        if f.id == flavor_id:
            return f
    raise KeyError(f"no flavor {flavor_id!r}")
