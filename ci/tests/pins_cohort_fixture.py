# SPDX-FileCopyrightText: 2026 Mattia Egloff <mattia.egloff@pm.me>
# SPDX-License-Identifier: GPL-3.0-or-later

import base64
import http.server
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from urllib.parse import parse_qs, unquote, urlsplit


OLD, NEW, CORE = "a" * 40, "b" * 40, "c" * 40
BRANCH = "chore/bump-core-0.106.1"
FILES = ("ci/pins.env", "Cargo.toml", "Cargo.lock")


class CohortFixture:
    def git(self, *args, cwd=None, check=True):
        return subprocess.run(
            ["git", "-c", "user.name=Pins Bot", "-c", "user.email=bot@example.invalid", *args],
            cwd=cwd or self.src, text=True, capture_output=True, check=check,
        ).stdout.strip()

    def __init__(self, entry, waiting=False, cli_only=False):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.src, self.remote, self.work = (self.root / p for p in ("src", "remote.git", "work"))
        self.src.mkdir()
        (self.src / "ci").mkdir()
        (self.src / ".cargo").mkdir()
        (self.src / ".cargo/config.toml").write_text("[net]\nretry = 1\n")
        (self.src / "Cargo.toml").write_text(
            '[package]\nname="vauchi-e2e-tests"\nversion="0.1.0"\n'
            '[dependencies]\nvauchi-core={git="https://gitlab.com/vauchi/core.git",'
            'tag="v0.106.1",version="=0.106.1"}\n'
            '[dev-dependencies]\nvauchi-relay={git="https://gitlab.com/vauchi/relay.git",rev="' + OLD + '"}\n'
        )
        (self.src / "Cargo.lock").write_text(
            'version=4\n[[package]]\nname="vauchi-e2e-tests"\nversion="0.1.0"\n'
            '[[package]]\nname="vauchi-core"\nversion="0.106.1"\n'
            'source="git+https://gitlab.com/vauchi/core.git?tag=v0.106.1#' + CORE + '"\n'
            '[[package]]\nname="vauchi-relay"\nversion="0.1.0"\n'
            'source="git+https://gitlab.com/vauchi/relay.git?rev=' + OLD + '#' + OLD + '"\n'
        )
        (self.src / "ci/pins.env").write_text(
            f"CORE_TAG=v0.106.1\nRELAY_SHA={OLD}\nCLI_SHA={OLD}\nOHTTP_RELAY_SHA={OLD}\nPREVIOUS_CLI_SHA=none\n"
        )
        updater = "#!/bin/sh\nset -eu\n[ -z \"${PROJECT_ACCESS_TOKEN:-}\" ] || exit 91\n"
        if waiting:
            updater += "exit 2\n"
        elif cli_only:
            updater += "python3 - <<'PY'\nfrom pathlib import Path\n"
            updater += f"p=Path('ci/pins.env'); p.write_text(p.read_text().replace('CLI_SHA={OLD}', 'CLI_SHA={NEW}'))\n"
            updater += "print('cli: moved old -> new')\nPY\n"
        else:
            updater += "python3 - <<'PY'\nfrom pathlib import Path\n"
            updater += f"for name in ('ci/pins.env', 'Cargo.toml'):\n p=Path(name); p.write_text(p.read_text().replace('{OLD}', '{NEW}'))\n"
            updater += "print('relay: moved old -> new')\nPY\n"
        (self.src / "ci/update-pins.sh").write_text(updater)
        (self.src / "unrelated.rs").write_text("original Rust work\n")
        self.git("init", "-q", "-b", "main")
        self.git("add", ".")
        self.git("commit", "-qm", "trusted main")
        self.main = self.git("rev-parse", "HEAD")
        self.git("checkout", "-qb", BRANCH)
        (self.src / "unrelated.rs").write_text("preserve branch Rust fixes\n")
        self.git("commit", "-qam", "branch work")
        self.git("clone", "-q", "--bare", str(self.src), str(self.remote))
        self.git("-C", str(self.remote), "symbolic-ref", "HEAD", "refs/heads/main")
        self.git("clone", "-q", str(self.remote), str(self.work))
        self.entry = entry
        self.sentinel, self.cargo_log = self.root / "branch-code-ran", self.root / "cargo.log"
        binary = self.root / "bin"
        binary.mkdir()
        cargo = binary / "cargo"
        cargo.write_text(
            f"#!{sys.executable}\nimport os\nfrom pathlib import Path\n"
            "assert 'PROJECT_ACCESS_TOKEN' not in os.environ, 'write token reached Cargo'\n"
            "assert 'CI_JOB_TOKEN' not in os.environ, 'job token reached Cargo'\n"
            "assert not (Path(os.environ['CARGO_HOME']) / 'credentials.toml').exists(), 'inherited Cargo credentials'\n"
            "assert 'BRANCH_CONTROLLED' not in Path('.cargo/config.toml').read_text()\n"
            f"Path({str(self.cargo_log)!r}).write_text('trusted Cargo update')\n"
            + ("" if cli_only else f"p=Path('Cargo.lock'); tag=dict(line.split('=',1) for line in Path('ci/pins.env').read_text().splitlines() if '=' in line)['CORE_TAG']; p.write_text(p.read_text().replace('{OLD}', '{NEW}').replace('v0.106.1',tag).replace('version=\"0.106.1\"','version=\"'+tag[1:]+'\"'))\n")
        )
        cargo.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update(PATH=str(binary) + os.pathsep + self.env["PATH"],
                        PROJECT_ACCESS_TOKEN="cohort-fixture", PINS_PUSH_URL=str(self.remote),
                        CI_JOB_TOKEN="job-fixture",
                        CI_PIPELINE_SOURCE="schedule", CI_COMMIT_REF_PROTECTED="true",
                        CI_PROJECT_ID="17", CI_DEFAULT_BRANCH="main", CI_COMMIT_SHA=self.main,
                        COHORT_BRANCH_SENTINEL=str(self.sentinel))
        self.open = True
        self.source_project_id = 17
        self.target_branch = "main"
        self.on_recheck = None
        self.pipeline_status = "running" if waiting else "success"
        self.pipeline_http_status = 200
        self.heads = {"relay": OLD if cli_only else NEW, "cli": NEW if cli_only else OLD, "ohttp-relay": OLD}
        self.pipeline_authentication = []
        self.component_tags = {"relay": "v0.106.1", "cli": "v0.106.1", "ohttp-relay": "v0.106.1"}
        self.core_protected = True
        self.core_tag_commits = [CORE]
        self.core_tag_reads = 0
        self.component_extra = {}
        self.file_metadata_override = {}
        self.component_main_protected = True
        self.posts = []
        self.on_post = None
        self.drop_reply = False
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                authenticated = self.headers.get("PRIVATE-TOKEN") == fixture.env["PROJECT_ACCESS_TOKEN"]
                if urlsplit(self.path).path.endswith("/pipelines"):
                    fixture.pipeline_authentication.append(authenticated)
                if not authenticated:
                    self.send_response(403)
                    self.end_headers()
                    self.wfile.write(b'{}')
                    return
                status, body = fixture.get(self.path)
                self.send_response(status)
                self.end_headers()
                self.wfile.write(json.dumps(body).encode())

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                fixture.posts.append(body)
                status, response = fixture.post(body)
                if fixture.drop_reply and status == 201:
                    self.close_connection = True
                    return
                self.send_response(status)
                self.end_headers()
                self.wfile.write(json.dumps(response).encode())

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.env["CI_API_V4_URL"] = f"http://127.0.0.1:{self.server.server_port}"

    def head(self):
        return self.git("--git-dir", str(self.remote), "rev-parse", BRANCH)

    def change(self, files):
        self.git("checkout", "-q", BRANCH)
        for name, value in files.items():
            path = self.src / name
            if path.is_symlink():
                path.unlink()
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(value)
        self.git("add", ".")
        self.git("commit", "-qm", "concurrent branch change")
        self.git("push", "-q", str(self.remote), f"HEAD:refs/heads/{BRANCH}")

    def read(self, name, ref=BRANCH):
        return self.git("--git-dir", str(self.remote), "show", f"{ref}:{name}")

    def read_bytes(self, name, ref=BRANCH):
        return subprocess.run(["git", "--git-dir", str(self.remote), "show", f"{ref}:{name}"],
                              capture_output=True, check=True).stdout

    def get(self, url):
        parsed = urlsplit(url)
        path, query = unquote(parsed.path), parse_qs(parsed.query)
        prefix = "/projects/vauchi/e2e"
        if path == prefix + "/merge_requests/7" and self.on_recheck:
            self.on_recheck()
        mr = dict(iid=7, state="opened" if self.open else "closed", target_branch=self.target_branch, source_branch=BRANCH,
                  source_project_id=self.source_project_id, target_project_id=17, sha=self.head())
        if path == prefix:
            return 200, {"id": 17, "default_branch": "main"}
        if path == prefix + "/merge_requests":
            return 200, [mr] if self.open else []
        if path == prefix + "/merge_requests/7":
            return 200, mr
        for repo, head in self.heads.items():
            if path == f"/projects/vauchi/{repo}/repository/branches/main":
                return 200, {"name": "main", "protected": self.component_main_protected, "commit": {"id": head}}
            if path == f"/projects/vauchi/{repo}/repository/files/Cargo.toml":
                content = (self.read("Cargo.toml", "main") + "\n").replace("0.106.1", self.component_tags[repo][1:])
                content += self.component_extra.get(repo, "")
                return 200, {"file_path": "Cargo.toml", "encoding": "base64",
                             "content": base64.b64encode(content.encode()).decode()}
        if "/repository/branches/" in path:
            branch = path.split("/repository/branches/", 1)[1]
            sha = self.main if branch == "main" else self.head()
            return 200, {"name": branch, "protected": branch == "main", "commit": {"id": sha}}
        if "/repository/files/" in path:
            name = path.split("/repository/files/", 1)[1]
            ref = query["ref"][0]
            content = self.read_bytes(name, ref)
            last = self.git("--git-dir", str(self.remote), "log", "-1", "--format=%H", ref, "--", name)
            return 200, {"file_path": name, "last_commit_id": last, "encoding": "base64",
                         "content": base64.b64encode(content).decode(), **self.file_metadata_override}
        if path.startswith("/projects/vauchi/core/repository/tags/"):
            commit = self.core_tag_commits[min(self.core_tag_reads, len(self.core_tag_commits) - 1)]
            self.core_tag_reads += 1
            return 200, {"protected": self.core_protected, "commit": {"id": commit}}
        if path.endswith("/pipelines"):
            return self.pipeline_http_status, [{"status": self.pipeline_status}]
        if path == "/user":
            return 200, {"name": "Pins Bot", "email": "bot@example.invalid"}
        return 404, {}

    def post(self, body):
        if self.on_post:
            self.on_post()
        if not self.git("--git-dir", str(self.remote), "show-ref", "--verify", "--quiet",
                        f"refs/heads/{BRANCH}", check=False):
            exists = subprocess.run(["git", "--git-dir", str(self.remote), "rev-parse", "--verify", BRANCH],
                                    capture_output=True).returncode == 0
            if not exists:
                return 400, {"message": "branch missing"}
        for action in body["actions"]:
            last = self.git("--git-dir", str(self.remote), "log", "-1", "--format=%H", BRANCH, "--", action["file_path"])
            if action["action"] != "update" or action["last_commit_id"] != last:
                return 409, {"message": "stale file"}
        checkout = self.root / "publish"
        self.git("clone", "-q", "--branch", BRANCH, str(self.remote), str(checkout))
        for action in body["actions"]:
            (checkout / action["file_path"]).write_bytes(base64.b64decode(action["content"]))
        self.git("commit", "-qam", body["commit_message"], cwd=checkout)
        self.git("push", "-q", str(self.remote), f"HEAD:refs/heads/{BRANCH}", cwd=checkout)
        return 201, {"id": self.head()}

    def run(self):
        return subprocess.run(["sh", str(self.entry)], cwd=self.work, env=self.env,
                              capture_output=True, text=True, timeout=30)

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.temp.cleanup()
