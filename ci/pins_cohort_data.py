# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

"""Validate the only branch-controlled data admitted to the cohort sandbox."""

import copy
import re
import tomllib
from urllib.parse import quote

CORE_URL = "https://gitlab.com/vauchi/core.git"
RELAY_URL = "https://gitlab.com/vauchi/relay.git"
FILES = ("ci/pins.env", "Cargo.toml", "Cargo.lock")
SHA = re.compile(r"[0-9a-f]{40}")
TAG = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+")


def require(condition, message):
    if not condition:
        raise ValueError(message)


def pins(data):
    result = {}
    for line in data.decode("utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        match = re.fullmatch(r"([A-Z_]+)=([a-zA-Z0-9._-]+)", line)
        require(match is not None, "pins must contain literal assignments only")
        key, value = match.groups()
        require(key not in result, "duplicate pin")
        result[key] = value
    require(set(result) == {"CORE_TAG", "RELAY_SHA", "CLI_SHA", "OHTTP_RELAY_SHA", "PREVIOUS_CLI_SHA"}, "unexpected pin keys")
    require(TAG.fullmatch(result["CORE_TAG"]), "invalid Core release tag")
    for key in ("RELAY_SHA", "CLI_SHA", "OHTTP_RELAY_SHA"):
        require(SHA.fullmatch(result[key]), "invalid component commit")
    require(result["PREVIOUS_CLI_SHA"] == "none" or SHA.fullmatch(result["PREVIOUS_CLI_SHA"]), "invalid previous-release commit")
    return result


def manifest(data, trusted, selected):
    actual = tomllib.loads(data.decode("utf-8"))
    expected = copy.deepcopy(tomllib.loads(trusted.decode("utf-8")))

    def update(table):
        if not isinstance(table, dict):
            return
        if table.get("git") == CORE_URL:
            table["tag"] = selected["CORE_TAG"]
            table["version"] = "=" + selected["CORE_TAG"][1:]
        elif table.get("git") == RELAY_URL:
            table["rev"] = selected["RELAY_SHA"]
        for value in table.values():
            update(value)

    update(expected)
    require(actual == expected, "branch manifest changes more than the approved Core and relay pins")
    return actual


def core_dependencies(document):
    entries = []

    def collect(table):
        if not isinstance(table, dict):
            return
        for name, value in table.items():
            if name in {"vauchi-core", "vauchi-app", "vauchi-protocol", "vauchi-platform"}:
                entries.append((name, value))
            elif isinstance(value, dict) and value.get("git") == CORE_URL:
                entries.append((value.get("package", name), value))
            collect(value)

    collect(document)
    return entries


def component_matches_core(data, tag):
    entries = core_dependencies(tomllib.loads(data.decode("utf-8")))
    return bool(entries) and all(isinstance(entry, dict) and entry.get("git") == CORE_URL and entry.get("tag") == tag
                                 and entry.get("version") == "=" + tag[1:] for _, entry in entries)


def lock(data, trusted, selected, core_commit, transitional=False, release_lock=None):
    actual = tomllib.loads(data.decode("utf-8"))
    baseline = tomllib.loads(trusted.decode("utf-8"))
    require(set(actual) == {"version", "package"} and actual["version"] == baseline["version"], "unsupported lockfile structure")
    require(isinstance(actual["package"], list), "invalid lock packages")
    baseline_packages = baseline["package"]
    allowed_sources = {p["source"] for p in baseline_packages if "source" in p
                       and not p["source"].startswith(("git+" + CORE_URL + "?", "git+" + RELAY_URL + "?"))}
    core_source = f"git+{CORE_URL}?tag={quote(selected['CORE_TAG'], safe='')}#{core_commit}"
    relay_source = f"git+{RELAY_URL}?rev={selected['RELAY_SHA']}#{selected['RELAY_SHA']}"
    allowed_sources.update((core_source, relay_source))
    if transitional:
        allowed_sources.update(p["source"] for p in baseline_packages if "source" in p)
    released = tomllib.loads(release_lock.decode("utf-8")) if release_lock is not None else {}
    released_packages = {(p["name"], p["version"], p["source"]): p.get("checksum")
                         for p in released.get("package", []) if "source" in p}
    dependency_sources = allowed_sources | {identity[2] for identity in released_packages}
    local = {(p["name"], p["version"]) for p in baseline_packages if "source" not in p}
    checksums = {(p["name"], p["version"], p.get("source")): p.get("checksum") for p in baseline_packages}
    seen = set()
    for package in actual["package"]:
        require(isinstance(package, dict) and set(package) <= {"name", "version", "source", "checksum", "dependencies"}, "unsupported lock package fields")
        require(isinstance(package.get("name"), str) and isinstance(package.get("version"), str), "invalid package identity")
        identity = (package["name"], package["version"], package.get("source"))
        require(identity not in seen, "duplicate lock package")
        seen.add(identity)
        source = package.get("source")
        if source is None:
            require(identity[:2] in local, "foreign source-less lock package")
        else:
            require(source in allowed_sources or identity in released_packages, "untrusted lockfile source")
            if source not in allowed_sources:
                require(package.get("checksum") == released_packages[identity], "changed checksum for a released package")
            if source.startswith("registry+"):
                checksum = package.get("checksum")
                require(isinstance(checksum, str) and re.fullmatch(r"[0-9a-f]{64}", checksum), "invalid registry checksum")
                if identity in checksums:
                    require(checksum == checksums[identity], "changed checksum for an existing registry package")
            if source == core_source and package["name"] in {"vauchi-core", "vauchi-app", "vauchi-protocol", "vauchi-platform"}:
                require(package["version"] == selected["CORE_TAG"][1:], "mixed Core lock versions")
        dependencies = package.get("dependencies", [])
        require(isinstance(dependencies, list) and all(isinstance(value, str) for value in dependencies), "invalid lock dependencies")
        for value in dependencies:
            if "://" in value:
                match = re.fullmatch(r"[^()]+ \(([^()]+)\)", value)
                require(match is not None and match[1] in dependency_sources, "untrusted source in a lock dependency identity")
    return actual


def validate(files, trusted, selected, core_commit, transitional=False, release_lock=None):
    document = manifest(files["Cargo.toml"], trusted["Cargo.toml"], selected)
    resolved = lock(files["Cargo.lock"], trusted["Cargo.lock"], selected, core_commit, transitional, release_lock)
    if not transitional:
        expected = {name for name, _ in core_dependencies(document)}
        found = {package["name"] for package in resolved["package"]
                 if package.get("source", "").startswith("git+" + CORE_URL + "?")}
        require(expected <= found, "resolver output omits required Core packages")
