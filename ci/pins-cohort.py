# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

"""Advance an existing cohort without executing any source-branch code."""

import base64
import http.client
import json
import os
import re
import runpy
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
from urllib.error import HTTPError, URLError
from urllib.parse import quote, urlencode
from urllib.request import Request, urlopen

# Cached runner bytecode is not trusted with the write token (#1210).
metadata = runpy.run_path(str(Path(__file__).with_name("pins_cohort_data.py")))
FILES, SHA = metadata["FILES"], metadata["SHA"]
component_matches_core = metadata["component_matches_core"]
pins, require, validate = metadata["pins"], metadata["require"], metadata["validate"]

PROJECT = "/projects/vauchi%2Fe2e"
LIMIT = 4 * 1024 * 1024


class UnknownCommitOutcome(RuntimeError):
    pass


class Client:
    def __init__(self):
        self.base = os.environ.get("CI_API_V4_URL", "https://gitlab.com/api/v4").rstrip("/")
        self.token = os.environ["PROJECT_ACCESS_TOKEN"]

    def request(self, path, body=None):
        method = "GET" if body is None else "POST"
        request = Request(self.base + path, method=method,
                          data=None if body is None else json.dumps(body).encode(),
                          headers={"PRIVATE-TOKEN": self.token, "Content-Type": "application/json"})
        try:
            with urlopen(request, timeout=30) as response:
                payload = response.read(2 * LIMIT + 1)
                require(len(payload) <= 2 * LIMIT, "API response exceeds metadata limit")
                return json.loads(payload)
        except HTTPError as error:
            raise RuntimeError(f"{method} request failed with HTTP {error.code}") from None
        except (URLError, http.client.HTTPException, TimeoutError, ConnectionError):
            if body is not None:
                raise UnknownCommitOutcome("commit response lost; do not blindly retry") from None
            raise RuntimeError("metadata request failed") from None


def git(*args, cwd=None):
    result = subprocess.run(["git", *args], cwd=cwd, env=child_environment(), capture_output=True, check=False)
    require(result.returncode == 0, "Git snapshot operation failed")
    return result.stdout


def blob(sha, name):
    mode = git("ls-tree", sha, "--", name).decode().split()
    require(len(mode) >= 4 and mode[0] == "100644" and mode[1] == "blob", "cohort metadata must be regular files")
    require(int(git("cat-file", "-s", f"{sha}:{name}")) <= LIMIT, "cohort metadata exceeds size limit")
    return git("show", f"{sha}:{name}")


def child_environment():
    # These programs see trusted-main code/configuration only; no write token is inherited.
    keys = ("PATH", "CARGO_HOME", "RUSTUP_HOME", "SCCACHE_DIR", "RUSTC_WRAPPER", "SSL_CERT_FILE", "SSL_CERT_DIR")
    return {key: os.environ[key] for key in keys if key in os.environ}


def main():
    require(os.environ.get("CI_PIPELINE_SOURCE") == "schedule" and
            os.environ.get("CI_COMMIT_REF_PROTECTED") == "true", "cohort writes require a protected scheduled job")
    client = Client()
    project = client.request(PROJECT)
    project_id = project["id"]
    require(type(project_id) is int and project_id > 0 and project["default_branch"] == "main", "unexpected target project")
    candidates = client.request(PROJECT + "/merge_requests?state=opened&per_page=100")
    require(isinstance(candidates, list), "invalid merge-request response")
    selected = next((mr for mr in candidates if isinstance(mr.get("source_branch"), str)
                     and mr["source_branch"].startswith("chore/bump-core-")), None)
    if selected is None:
        print("cohort: no open Core bump MR")
        return
    require(selected.get("source_project_id") == project_id == selected.get("target_project_id")
            and selected.get("state") == "opened" and selected.get("target_branch") == "main", "cohort must be an open same-project MR targeting main")
    sha, branch, iid = selected.get("sha"), selected["source_branch"], selected.get("iid")
    require(isinstance(sha, str) and SHA.fullmatch(sha) and type(iid) is int and iid > 0, "invalid cohort identity")
    branch_path = PROJECT + "/repository/branches/" + quote(branch, safe="")
    branch_data = client.request(branch_path)
    require(branch_data.get("name") == branch and branch_data.get("commit", {}).get("id") == sha, "cohort head moved before snapshot capture")
    main_data = client.request(PROJECT + "/repository/branches/main")
    main_sha = main_data.get("commit", {}).get("id")
    require(main_data.get("name") == "main" and main_data.get("protected") is True
            and isinstance(main_sha, str) and SHA.fullmatch(main_sha), "main is not a protected trusted snapshot")
    git("fetch", "--quiet", "--no-tags", "origin", main_sha, sha)
    trusted = {name: blob(main_sha, name) for name in FILES}
    source = {name: blob(sha, name) for name in FILES}
    metadata = {}
    for name in FILES:
        response = client.request(PROJECT + "/repository/files/" + quote(name, safe="") + "?" + urlencode({"ref": sha}))
        require(response.get("file_path") == name and response.get("encoding") == "base64", "invalid file metadata")
        last = response.get("last_commit_id")
        require(isinstance(last, str) and SHA.fullmatch(last), "invalid file revision")
        require(base64.b64decode(response["content"], validate=True) == source[name], "file metadata does not match captured snapshot")
        metadata[name] = last

    core_tags = {}

    def validate_set(files, transitional=False):
        selected_pins = pins(files["ci/pins.env"])
        tag = client.request("/projects/vauchi%2Fcore/repository/tags/" + quote(selected_pins["CORE_TAG"], safe=""))
        core_commit = tag.get("commit", {}).get("id")
        require(tag.get("protected") is True and isinstance(core_commit, str) and SHA.fullmatch(core_commit), "Core tag lacks protected release authority")
        require(core_tags.setdefault(selected_pins["CORE_TAG"], core_commit) == core_commit, "Core release tag moved during validation")
        trusted_pins = pins(trusted["ci/pins.env"])
        for repo, key in (("relay", "RELAY_SHA"), ("cli-display", "CLI_SHA"), ("ohttp-relay", "OHTTP_RELAY_SHA")):
            if selected_pins[key] != trusted_pins[key]:
                runs = client.request("/projects/vauchi%2F" + repo + "/pipelines?" + urlencode({"ref": "main", "sha": selected_pins[key], "per_page": 1}))
                require(isinstance(runs, list) and runs and runs[0].get("status") == "success", "component revision lacks successful main validation")
        validate(files, trusted, selected_pins, core_commit, transitional)

    validate_set(source, transitional=True)
    with tempfile.TemporaryDirectory(prefix="pins-cohort-") as temporary:
        root = Path(temporary)
        snapshot = root / "main"
        snapshot.mkdir()
        archive = root / "main.tar"
        archive.write_bytes(git("archive", main_sha))
        with tarfile.open(archive) as contents:
            contents.extractall(snapshot, filter="data")
        for name, content in source.items():
            (snapshot / name).write_bytes(content)
        environment = child_environment()
        environment["CARGO_HOME"] = str(root / "cargo-home")
        selected_pins = pins(source["ci/pins.env"])
        moves = []
        for repo, key, core_pinned in (("relay", "RELAY_SHA", True), ("cli-display", "CLI_SHA", True), ("ohttp-relay", "OHTTP_RELAY_SHA", False)):
            component = "/projects/vauchi%2F" + repo
            main = client.request(component + "/repository/branches/main")
            head = main.get("commit", {}).get("id")
            require(main.get("name") == "main" and main.get("protected") is True
                    and isinstance(head, str) and SHA.fullmatch(head), "invalid protected component main")
            previous = selected_pins[key]
            if head == previous:
                continue
            runs = client.request(component + "/pipelines?" + urlencode({"ref": "main", "sha": head, "per_page": 1}))
            require(isinstance(runs, list), "invalid component pipeline response")
            if not runs or runs[0].get("status") != "success":
                print(f"cohort: {repo} waits for a successful main pipeline")
                continue
            if core_pinned:
                response = client.request(component + "/repository/files/Cargo.toml?" + urlencode({"ref": head}))
                require(response.get("file_path") == "Cargo.toml" and response.get("encoding") == "base64", "invalid component manifest response")
                content = base64.b64decode(response["content"], validate=True)
                if not component_matches_core(content, selected_pins["CORE_TAG"]):
                    print(f"cohort: {repo} waits for its Core bump")
                    continue
            pin_file = snapshot / "ci/pins.env"
            pattern = rb"(?m)^([ \t]*" + key.encode() + rb"=)" + previous.encode() + rb"(?=[ \t]*$)"
            content, count = re.subn(pattern, lambda match: match[1] + head.encode(), pin_file.read_bytes())
            require(count == 1, "component pin could not be updated literally")
            pin_file.write_bytes(content)
            if key == "RELAY_SHA":
                path = snapshot / "Cargo.toml"
                path.write_bytes(path.read_bytes().replace(previous.encode(), head.encode()))
            selected_pins[key] = head
            moves.append(f"{repo}: moved {previous} -> {head}")
        if not moves:
            print("cohort: component mains are not ready; nothing published")
            return
        for repo, key in (("relay", "RELAY_SHA"), ("cli-display", "CLI_SHA")):
            response = client.request("/projects/vauchi%2F" + repo + "/repository/files/Cargo.toml?" + urlencode({"ref": selected_pins[key]}))
            require(response.get("file_path") == "Cargo.toml" and response.get("encoding") == "base64", "invalid cohort manifest response")
            if not component_matches_core(base64.b64decode(response["content"], validate=True), selected_pins["CORE_TAG"]):
                print(f"cohort: waits until {repo} shares the Core release")
                return
        for move in moves:
            print(move)
        subprocess.run(["cargo", "update", "-p", "vauchi-relay"], cwd=snapshot, env=environment, check=True)
        generated = {name: (snapshot / name).read_bytes() for name in FILES}
        validate_set(generated)
        require(any(generated[name] != source[name] for name in FILES), "updater reported changes without moving cohort metadata")
        actions = [{"action": "update", "file_path": name, "encoding": "base64",
                    "content": base64.b64encode(content).decode(), "last_commit_id": metadata[name]}
                   for name, content in generated.items()]
        current = client.request(PROJECT + f"/merge_requests/{iid}")
        require(current.get("state") == "opened" and current.get("sha") == sha
                and current.get("source_branch") == branch and current.get("target_branch") == "main"
                and current.get("source_project_id") == project_id == current.get("target_project_id"),
                "cohort MR moved before publication")
        body = {"branch": branch, "commit_message": "chore: the Core-bump cohort follows green component mains\n\n" + "\n".join(moves)
                + '\n\nTools used: "GitLab CI (GitLab)"',
                "actions": actions}
        try:
            response = client.request(PROJECT + "/repository/commits", body)
            require(isinstance(response.get("id"), str) and SHA.fullmatch(response["id"]), "invalid published commit response")
        except UnknownCommitOutcome:
            reconciled = client.request(branch_path)
            reconciled_sha = reconciled.get("commit", {}).get("id")
            require(reconciled.get("name") == branch and isinstance(reconciled_sha, str)
                    and SHA.fullmatch(reconciled_sha), "commit outcome unknown; invalid reconciliation snapshot")
            for action in actions:
                response = client.request(PROJECT + "/repository/files/" + quote(action["file_path"], safe="") + "?" + urlencode({"ref": reconciled_sha}))
                require(response.get("encoding") == "base64" and base64.b64decode(response["content"], validate=True) == generated[action["file_path"]],
                        "commit outcome unknown; inspect the branch before retrying")
            print("cohort: desired files confirmed after lost response; no duplicate write")
        print(f"cohort: advanced {branch}; its open MR's pipeline reruns")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError, AttributeError, RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"cohort: ERROR: {error}", file=sys.stderr)
        sys.exit(1)
