"""T37: assemble and scrub the recorded responses into fixture files.

Usage: python3 -I assemble.py <raw dir> <out dir>
One map from real value to stand-in, shared by every file. Only whole fields
are replaced, plus known real values inside URLs and error messages. Enum
values, times, counts, nulls, key order and shape stay as GitHub sent them.
"""
import json
import re
import sys
from pathlib import Path

raw, out = Path(sys.argv[1]), Path(sys.argv[2])

CASES = [
    # (raw suffix, fixture name, request)
    ("1", "poll", {"find": True, "nodes": ["record", "record"], "lookup": False}),
    ("2", "first-refresh", {"find": False, "nodes": ["record_and_items"], "lookup": False}),
    ("3", "checks", {"find": False, "nodes": ["record"], "lookup": False}),
    ("4", "merged-and-closed", {"find": False, "nodes": ["record", "record"], "lookup": False}),
    ("5", "null-rollup", {"find": False, "nodes": ["record"], "lookup": False}),
    ("6", "unknown-id", {"find": False, "nodes": ["record", "record"], "lookup": False}),
    ("7a", "lookup-found", {"find": False, "nodes": [], "lookup": True}),
    ("7b", "lookup-no-pr", {"find": False, "nodes": [], "lookup": True}),
    ("7c", "lookup-no-repo", {"find": False, "nodes": [], "lookup": True}),
    # The 401 answered body-1, the poll.
    ("401", "bad-credentials", {"find": True, "nodes": ["record", "record"], "lookup": False}),
]
KEEP = re.compile(r"^(date|retry-after|x-ratelimit-.*)$")


def split(path):
    text = path.read_text()
    head, _, body = re.split(r"(\r?\n\r?\n)", text, maxsplit=1)
    lines = head.splitlines()
    status = int(lines[0].split()[1])
    headers = {}
    for line in lines[1:]:
        k, _, v = line.partition(":")
        k = k.strip().lower()
        if KEEP.match(k):
            headers[k] = v.strip()
    return status, headers, json.loads(body)


class Map:
    def __init__(self, fmt):
        self.fmt, self.m = fmt, {}

    def __call__(self, real):
        if real not in self.m:
            self.m[real] = self.fmt(len(self.m) + 1)
        return self.m[real]


ids = Map(lambda n: f"PR_scrubbed{n:04d}")
logins = Map(lambda n: f"user{n}")
titles = Map(lambda n: f"Title {n}")
branches = Map(lambda n: f"branch-{n}")
shas = Map(lambda n: f"{n:040x}")
nums = Map(lambda n: 1000 + n)
cursors = Map(lambda n: f"cursor{n}")
repos = {}  # "owner/name" -> stand-in


def repo(nwo):
    if nwo not in repos:
        owner, name = nwo.split("/")
        repos[nwo] = f"{logins(owner)}/repo{len(repos) + 1}"
    return repos[nwo]


def url(u):
    for real, fake in repos.items():
        u = u.replace(f"/{real}/", f"/{fake}/")
    return re.sub(r"(?<=/)\d{5,}(?=/|$)", lambda m: str(nums(int(m.group()))), u)


def message(s):
    s = re.sub(r"PR_kw[A-Za-z0-9_-]+", lambda m: ids(m.group()), s)
    for real, fake in sorted(KNOWN.items(), key=lambda kv: -len(kv[0])):
        s = s.replace(real, fake)
    return s


def walk(v, key=None):
    if isinstance(v, dict):
        return {k: walk(x, k) for k, x in v.items()}
    if isinstance(v, list):
        return [walk(x, key) for x in v]
    if v is None:
        return v
    if key == "id":
        return ids(v)
    if key == "login":
        return logins(v)
    if key == "title":
        return titles(v)
    if key in ("baseRefName", "headRefName"):
        return branches(v)
    if key == "headRefOid":
        return shas(v)
    if key == "databaseId":
        return nums(v)
    if key == "endCursor":
        return cursors(v)
    if key == "nameWithOwner":
        return repo(v)
    if key in ("url", "detailsUrl", "targetUrl"):
        return url(v)
    if key == "message":
        return message(v)
    return v


# Every repository name is known before any URL or message is scrubbed.
loaded = []
for suffix, name, request in CASES:
    status, headers, body = split(raw / f"raw-{suffix}.txt")
    loaded.append((name, request, status, headers, body))
    for v in re.findall(r'"nameWithOwner": ?"([^"]+)"', json.dumps(body)):
        repo(v)
# The lookups' variables name a repository too (7c's doesn't exist).
for suffix in ("7a", "7b", "7c"):
    var = json.loads((raw / f"body-{suffix}.json").read_text())["variables"]
    repo(f"{var['owner']}/{var['name']}")
KNOWN = dict(repos)
for nwo, fake in repos.items():
    (o, n), (fo, fn) = nwo.split("/"), fake.split("/")
    KNOWN[o], KNOWN[n] = fo, fn

out.mkdir(parents=True, exist_ok=True)
for name, request, status, headers, body in loaded:
    fixture = {"request": request, "status": status, "headers": headers, "body": walk(body)}
    (out / f"recorded-{name}.json").write_text(json.dumps(fixture, indent=2) + "\n")
    print(f"recorded-{name}.json")

# The map stays in the scratch directory, never in the repository.
(raw / "scrub-map.json").write_text(json.dumps({
    "ids": ids.m, "logins": logins.m, "titles": titles.m, "branches": branches.m,
    "shas": shas.m, "nums": {str(k): v for k, v in nums.m.items()},
    "cursors": cursors.m, "repos": repos,
}, indent=2))
