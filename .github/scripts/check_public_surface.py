#!/usr/bin/env python3
"""Check that the public surface of this repository stays private-safe.

Scans commit messages, tag messages, tracked files, staged files, and release
assets for private-document references, local environment narration, local
absolute paths, verification trailers, email leakage, secret-like strings, and
unapproved CJK. The repository is private by default; this gate runs before any
open-source decision and before publishing release assets.
"""

from __future__ import annotations

import argparse
import fnmatch
import getpass
import os
import re
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

PRIVATE_DOC = re.compile(
    r"(?:(?<![\w-])(?!README\b|THIRD_PARTY_NOTICES\b|LICENSE\b)"
    r"[A-Z][A-Z0-9_-]{2,}\.md\b|docs/(?!public/)[^/\s]+\.md|§)"
)
ENV_NARRATION = re.compile(
    r"(?<!-)(WSL|local host|workstation|Git Bash|/mnt/d|"
    r"does not compile on|MSVC not installed)(?!-)",
    re.IGNORECASE,
)
VERIFICATION = re.compile(r"^\s*Verification:\s*", re.IGNORECASE | re.MULTILINE)
LOCAL_PATH = re.compile(
    r"([A-Za-z]:\\Users\\|/home/|/Users/|/mnt/|\\\\wsl)"
)
EMAIL = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
CJK = re.compile(
    r"[\u2e80-\u2eff\u3000-\u303f\u31c0-\u31ef\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff\uff00-\uffef]"
)
SECRET = re.compile(
    r"(ghp_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,}|"
    r"AKIA[0-9A-Z]{16}|xox[bp]-[A-Za-z0-9-]{10,}|"
    r"BEGIN [A-Z ]*PRIVATE KEY|sk-[A-Za-z0-9]{20,}|"
    r"\b\d{8,10}:[A-Za-z0-9_-]{30,}\b|"
    r"(?:api[_-]?key|secret|token|password)\s*[:=]\s*['\"][^'\"]{8,}['\"])",
    re.IGNORECASE,
)
TREE_ALLOW = {
    "README.md": {"cjk"},
    "README.*.md": {"cjk"},
    "THIRD_PARTY_NOTICES.md": {"email"},
    "**/*.zh-CN.md": {"cjk"},
    "**/*.zh_CN.md": {"cjk"},
    "locales/**": {"cjk"},
    "i18n/**": {"cjk"},
    "**/locales/**": {"cjk"},
    "**/i18n/**": {"cjk"},
    ".github/scripts/check_public_surface.py": {"all"},
}
LANG_ALLOWLIST = ".github/language_allowlist.txt"
BINARY_PATTERNS = [
    (b"\\\\wsl", "local WSL path"),
    (b"Verification:", "verification trailer"),
]
PRINTABLE = re.compile(rb"[\x20-\x7e]{6,}")
USER_PATH = re.compile(
    r"(?:C:\\Users\\(?!(?:runner|runneradmin|Public|Default|All Users)\\)[A-Za-z0-9_.-]+"
    r"|/home/(?!(?:runner)/)[A-Za-z0-9_.-]+"
    r"|/Users/(?!(?:runner)/)[A-Za-z0-9_.-]+"
    r"|/mnt/[a-z]/[A-Za-z0-9_.-]+)"
)
PERSONAL_RE: re.Pattern[str] | None = None
PERSONAL_NEEDLES: list[tuple[bytes, str]] = []


def run_git(repo: Path, args: list[str]) -> str:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return result.stdout.decode("utf-8", "replace")


def run_git_bytes(repo: Path, args: list[str]) -> bytes:
    result = subprocess.run(
        ["git", "-C", str(repo), *args],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=True,
    )
    return result.stdout


def load_personal(repo: Path) -> tuple[re.Pattern[str] | None, list[tuple[bytes, str]]]:
    names = {
        getpass.getuser(),
        os.environ.get("USER", ""),
        os.environ.get("USERNAME", ""),
    }
    try:
        email = run_git(repo, ["config", "user.email"]).strip()
    except subprocess.CalledProcessError:
        email = ""
    if email and "@" in email:
        local_full = email.split("@", 1)[0]
        names.add(local_full)
        if "+" in local_full:
            prefix = local_full.split("+", 1)[0]
            if len(prefix) >= 5:
                names.add(prefix)
    names = {name.strip() for name in names if name and len(name.strip()) >= 3}
    if not names:
        return None, []
    pattern = re.compile("|".join(re.escape(name) for name in sorted(names)), re.IGNORECASE)
    needles = [(name.encode("utf-8"), "personal identifier") for name in sorted(names)]
    return pattern, needles


def load_language_allowlist(repo: Path) -> set[str]:
    path = repo / LANG_ALLOWLIST
    if not path.exists():
        return set()
    allowed: set[str] = set()
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if line:
            allowed.add(line.replace("\\", "/"))
    return allowed


def load_repo_allowlist(path: Path) -> list[str]:
    if not path.exists():
        return []
    patterns: list[str] = []
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.split("#", 1)[0].strip()
        if line:
            patterns.append(line.replace("\\", "/"))
    return patterns


def repo_path_allowed(path: str, patterns: list[str]) -> bool:
    for pattern in patterns:
        if pattern.endswith("/"):
            if path.startswith(pattern):
                return True
        elif fnmatch.fnmatchcase(path, pattern):
            return True
    return False


def allow_for(path: str, language_allowlist: set[str]) -> set[str]:
    allow: set[str] = set()
    for pattern, allowed in TREE_ALLOW.items():
        if fnmatch.fnmatchcase(path, pattern):
            allow |= allowed
    if path in language_allowlist:
        allow.add("cjk")
    return allow


def report(kind: str, path: str, number: int, text: str, name: str, errors: list[str]) -> None:
    errors.append(f"{kind} {path}:{number}: {name}: {text.strip()[:160]}")


def scan_text(
    kind: str,
    path: str,
    text: str,
    allow: set[str],
    errors: list[str],
) -> None:
    if "all" in allow:
        return
    for number, line in enumerate(text.splitlines(), 1):
        checks = [
            ("private-doc", PRIVATE_DOC),
            ("env-narration", ENV_NARRATION),
            ("local-path", LOCAL_PATH),
            ("secret", SECRET),
        ]
        if PERSONAL_RE is not None:
            checks.append(("personal", PERSONAL_RE))
        for name, pattern in checks:
            if name not in allow and pattern.search(line):
                report(kind, path, number, line, name, errors)
        if "email" not in allow:
            for match in EMAIL.finditer(line):
                value = match.group(0).lower()
                if value.endswith("users.noreply.github.com"):
                    continue
                if re.search(r"@example\.(com|org|net)$", value):
                    continue
                report(kind, path, number, line, "email", errors)
        if "cjk" not in allow and CJK.search(line):
            report(kind, path, number, line, "cjk", errors)
        if VERIFICATION.search(line):
            report(kind, path, number, line, "verification-tail", errors)


def scan_message(path: str, text: str, errors: list[str]) -> None:
    scan_text("message", path, text, set(), errors)


def scan_range(repo: Path, rev_range: str, errors: list[str]) -> None:
    if not rev_range or ".." not in rev_range or rev_range.startswith(".."):
        text = run_git(repo, ["log", "-1", "--format=%B"])
        scan_message("HEAD", text, errors)
        return
    try:
        text = run_git(repo, ["log", "--no-merges", "--format=%B%x1e", rev_range])
    except subprocess.CalledProcessError:
        text = run_git(repo, ["log", "-1", "--format=%B"])
        scan_message("HEAD", text, errors)
        return
    for index, body in enumerate(text.split("\x1e"), 1):
        if body.strip():
            scan_message(f"{rev_range}#{index}", body, errors)


def scan_tree(
    repo: Path,
    language_allowlist: set[str],
    repo_allowlist: list[str],
    errors: list[str],
) -> None:
    for name in run_git(repo, ["ls-files", "-z"]).split("\x00"):
        if not name:
            continue
        check_path(name, repo_allowlist, errors)
        path = repo / name
        if not path.is_file():
            continue
        data = path.read_bytes()
        if b"\x00" in data[:4096]:
            continue
        scan_text(
            "tree",
            name,
            data.decode("utf-8", "replace"),
            allow_for(name, language_allowlist),
            errors,
        )


def check_path(name: str, repo_allowlist: list[str], errors: list[str]) -> None:
    if repo_allowlist and not repo_path_allowed(name, repo_allowlist):
        errors.append(f"tree {name}: path outside repo allowlist")
    if CJK.search(name):
        errors.append(f"tree {name}: cjk in path")
    if LOCAL_PATH.search(name):
        errors.append(f"tree {name}: local path in name")
    if PERSONAL_RE is not None and PERSONAL_RE.search(name):
        errors.append(f"tree {name}: personal identifier in path")


def scan_staged(
    repo: Path,
    language_allowlist: set[str],
    repo_allowlist: list[str],
    errors: list[str],
) -> None:
    for name in run_git(repo, ["diff", "--cached", "--name-only", "-z"]).split("\x00"):
        if not name:
            continue
        check_path(name, repo_allowlist, errors)
        try:
            data = run_git_bytes(repo, ["show", f":{name}"])
        except subprocess.CalledProcessError:
            continue
        if b"\x00" in data[:4096]:
            continue
        scan_text(
            "staged",
            name,
            data.decode("utf-8", "replace"),
            allow_for(name, language_allowlist),
            errors,
        )


def scan_tag(repo: Path, tag: str, errors: list[str]) -> None:
    text = run_git(repo, ["tag", "-l", "-n99", tag])
    scan_text("tag", f"tag:{tag}", text, {"cjk"}, errors)


def scan_assets(root: Path, errors: list[str]) -> None:
    if not root.exists():
        errors.append(f"asset {root}: path not found")
        return
    if root.is_file():
        scan_asset_file(root, errors)
        return
    for path in root.rglob("*"):
        if path.is_file():
            scan_asset_file(path, errors)


def scan_asset_file(path: Path, errors: list[str]) -> None:
    suffix = path.suffix.lower()
    if suffix == ".zip":
        try:
            with zipfile.ZipFile(path) as archive:
                for entry in archive.infolist():
                    scan_binary(path, entry.filename, archive.read(entry), errors)
        except (zipfile.BadZipFile, OSError) as exc:
            errors.append(f"asset {path}: cannot read zip: {exc}")
    elif suffix in {".gz", ".tgz"} and path.name.endswith((".tar.gz", ".tgz")):
        try:
            with tarfile.open(path, "r:*") as archive:
                for member in archive.getmembers():
                    if member.isfile():
                        data = archive.extractfile(member).read()
                        scan_binary(path, member.name, data, errors)
        except (tarfile.TarError, OSError) as exc:
            errors.append(f"asset {path}: cannot read archive: {exc}")
    else:
        try:
            data = path.read_bytes()
        except OSError as exc:
            errors.append(f"asset {path}: cannot read: {exc}")
            return
        scan_binary(path, "", data, errors)


def scan_binary(path: Path, entry: str, data: bytes, errors: list[str]) -> None:
    label = f"{path}!{entry}" if entry else str(path)
    for needle, reason in BINARY_PATTERNS + PERSONAL_NEEDLES:
        if needle in data:
            errors.append(f"asset {label}: {reason}: {needle.decode(errors='replace')}")
    for run in PRINTABLE.finditer(data):
        text = run.group().decode("ascii", "ignore")
        if USER_PATH.search(text):
            errors.append(f"asset {label}: local user path: {text[:120]}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", default=".")
    parser.add_argument("--commit-msg")
    parser.add_argument("--range")
    parser.add_argument("--tag")
    parser.add_argument("--tree", action="store_true")
    parser.add_argument("--staged", action="store_true")
    parser.add_argument("--assets", action="append")
    parser.add_argument("--personal", action="store_true")
    parser.add_argument("--repo-allowlist", default="")
    args = parser.parse_args()

    repo = Path(args.repo).resolve()
    language_allowlist = load_language_allowlist(repo)
    repo_allowlist = (
        load_repo_allowlist(Path(args.repo_allowlist)) if args.repo_allowlist else []
    )
    errors: list[str] = []
    global PERSONAL_RE, PERSONAL_NEEDLES
    if args.personal:
        PERSONAL_RE, PERSONAL_NEEDLES = load_personal(repo)

    if not any([args.commit_msg, args.range, args.tag, args.tree, args.staged, args.assets]):
        args.tree = True

    if args.commit_msg:
        text = Path(args.commit_msg).read_text(encoding="utf-8")
        scan_message(args.commit_msg, text, errors)
    if args.range:
        scan_range(repo, args.range, errors)
    if args.tag:
        scan_tag(repo, args.tag, errors)
    if args.tree:
        scan_tree(repo, language_allowlist, repo_allowlist, errors)
    if args.staged:
        scan_staged(repo, language_allowlist, repo_allowlist, errors)
    for asset in args.assets or []:
        scan_assets(Path(asset), errors)

    if errors:
        for error in errors:
            print(f"ERROR {error}")
        print(f"public_surface: {len(errors)} finding(s)")
        return 1
    print("public_surface: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
