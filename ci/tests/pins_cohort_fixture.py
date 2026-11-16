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

    def __init__(self, entry, waiting=False):
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
            "assert 'BRANCH_CONTROLLED' not in Path('.cargo/config.toml').read_text()\n"
            f"Path({str(self.cargo_log)!r}).write_text('trusted Cargo update')\n"
            f"p=Path('Cargo.lock'); p.write_text(p.read_text().replace('{OLD}', '{NEW}'))\n"
        )
        cargo.chmod(0o755)
        self.env = os.environ.copy()
        self.env.update(PATH=str(binary) + os.pathsep + self.env["PATH"],
                        PROJECT_ACCESS_TOKEN="cohort-fixture", PINS_PUSH_URL=str(self.remote),
                        CI_PIPELINE_SOURCE="schedule", CI_COMMIT_REF_PROTECTED="true",
                        CI_PROJECT_ID="17", CI_DEFAULT_BRANCH="main", CI_COMMIT_SHA=self.main,
                        COHORT_BRANCH_SENTINEL=str(self.sentinel))
        self.open = True
        self.source_project_id = 17
        self.posts = []
        self.on_post = None
        self.drop_reply = False
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
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

    def get(self, url):
        parsed = urlsplit(url)
        path, query = unquote(parsed.path), parse_qs(parsed.query)
        prefix = "/projects/vauchi/e2e"
        mr = dict(iid=7, state="opened", target_branch="main", source_branch=BRANCH,
                  source_project_id=self.source_project_id, target_project_id=17, sha=self.head())
        if path == prefix:
            return 200, {"id": 17, "default_branch": "main"}
        if path == prefix + "/merge_requests":
            return 200, [mr] if self.open else []
        if path == prefix + "/merge_requests/7":
            return 200, mr
        if "/repository/branches/" in path:
            branch = path.split("/repository/branches/", 1)[1]
            sha = self.main if branch == "main" else self.head()
            return 200, {"name": branch, "protected": branch == "main", "commit": {"id": sha}}
        if "/repository/files/" in path:
            name = path.split("/repository/files/", 1)[1]
            ref = query["ref"][0]
            content = self.read(name, ref) + "\n"
            last = self.git("--git-dir", str(self.remote), "log", "-1", "--format=%H", ref, "--", name)
            return 200, {"file_path": name, "last_commit_id": last, "encoding": "base64",
                         "content": base64.b64encode(content.encode()).decode()}
        if path.startswith("/projects/vauchi/core/repository/tags/"):
            return 200, {"commit": {"id": CORE}}
        if path.endswith("/pipelines"):
            return 200, [{"status": "success"}]
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

